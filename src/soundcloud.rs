//! Клиент SoundCloud с двумя режимами.
//!
//! **Вход через сайт** (основной): пользователь входит на странице SoundCloud в окне
//! Nebula (см. `sc_login`), мы сохраняем токен его сессии и client_id веб-клиента и
//! обращаемся к тому же API, что и сайт (`api-v2.soundcloud.com`). Ключи и подписки не нужны.
//!
//! **Свои ключи API** (для разработчиков): официальный API (https://api.soundcloud.com)
//! по OAuth 2.1. Публичный каталог — токен приложения по client credentials (кэшируется на
//! диске: лимит SoundCloud — 50 токенов за 12 часов), личные данные — Authorization Code + PKCE
//! с приёмом кода на локальном порту. Ключи выдаются на https://soundcloud.com/you/apps.
//!
//! Если есть веб-сессия, используется она. Всё хранится только локально в `soundcloud.json`.

use crate::audio::MediaSpec;
use crate::model::{Page, PlaylistInfo, Track, UserInfo};
use crate::paths::{now_secs, paths, read_json, write_json};
use crate::settings::StreamQuality;
use base64::Engine as _;
use parking_lot::Mutex;
use reqwest::header::{ACCEPT, AUTHORIZATION};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const API: &str = "https://api.soundcloud.com";
const API_V2: &str = "https://api-v2.soundcloud.com";
const API_V2_HOST: &str = "api-v2.soundcloud.com";
const TOKEN_URL: &str = "https://secure.soundcloud.com/oauth/token";
const AUTH_URL: &str = "https://secure.soundcloud.com/authorize";
pub const DEFAULT_REDIRECT: &str = "http://127.0.0.1:47823/callback";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: u64,
}

impl Token {
    fn fresh(&self) -> bool {
        now_secs() + 60 < self.expires_at
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct ScStore {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub app_token: Option<Token>,
    pub user_token: Option<Token>,
    pub user: Option<UserInfo>,
    /// Вход через сайт: токен сессии пользователя и client_id веб-клиента SoundCloud.
    pub web_token: Option<String>,
    pub web_client_id: Option<String>,
    /// Токен сессии, из которой пользователь вышел или которая истекла: его cookie может
    /// ещё жить в окне входа, такой токен повторно не принимаем.
    pub banned_token: Option<String>,
    /// Перед следующим входом стереть данные профиля окна входа.
    pub clear_on_login: bool,
}

pub struct SoundCloud {
    http: reqwest::Client,
    store: Mutex<ScStore>,
    token_lock: tokio::sync::Mutex<()>,
    expired: AtomicBool,
}

fn store_path() -> std::path::PathBuf {
    paths().config.join("soundcloud.json")
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn u(v: &Value, k: &str) -> Option<u64> {
    v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f as u64)))
}

fn urn_of(v: &Value, kind: &str) -> Option<String> {
    s(v, "urn").or_else(|| {
        v.get("id").and_then(|id| {
            id.as_u64()
                .map(|n| format!("soundcloud:{kind}:{n}"))
                .or_else(|| id.as_str().map(|s| if s.contains(':') { s.to_string() } else { format!("soundcloud:{kind}:{s}") }))
        })
    })
}

/// Доступность трека: официальный API отдаёт `access`, API сайта — `policy` и `streamable`.
fn access_of(v: &Value) -> Option<String> {
    if v.get("streamable") == Some(&Value::Bool(false)) {
        return Some("blocked".into());
    }
    s(v, "access").or_else(|| {
        s(v, "policy").map(|p| match p.as_str() {
            "SNIP" => "preview".to_string(),
            "BLOCK" => "blocked".to_string(),
            _ => "playable".to_string(),
        })
    })
}

/// Числовой id из URN (`soundcloud:tracks:123` → `123`) — пути API сайта используют его.
pub fn sc_id(urn: &str) -> &str {
    urn.rsplit(':').next().unwrap_or(urn)
}

/// Адрес API сайта с client_id: для путей и для `next_href`, где client_id нет.
/// Принимает только `api-v2.soundcloud.com` — токен не уйдёт на чужой хост.
pub fn v2_url(path_or_url: &str, params: &[(&str, &str)], client_id: &str) -> Result<Url, String> {
    let mut url = if path_or_url.starts_with("http") {
        Url::parse(path_or_url)
    } else {
        Url::parse(&format!("{API_V2}{path_or_url}"))
    }
    .map_err(|e| format!("адрес API: {e}"))?;
    if url.scheme() != "https" || url.host_str() != Some(API_V2_HOST) {
        return Err("неожиданный адрес API SoundCloud".into());
    }
    let mut pairs: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| k != "client_id").map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    for (k, v) in params {
        pairs.retain(|(pk, _)| pk != k);
        pairs.push((k.to_string(), v.to_string()));
    }
    pairs.push(("client_id".into(), client_id.into()));
    url.query_pairs_mut().clear().extend_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    Ok(url)
}

/// Выбранный поток трека в API сайта.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    pub url: String,
    pub hls: bool,
    pub preview: bool,
}

/// Выбор потока из `media.transcodings`: AAC HLS → MP3 HLS → прогрессивный MP3.
/// Зашифрованные (DRM) и Opus-потоки пропускаем: их нельзя или нечем декодировать.
pub fn pick_transcoding(v: &Value, quality: StreamQuality) -> Option<Pick> {
    let list = v.get("media")?.get("transcodings")?.as_array()?;
    let policy_snip = s(v, "policy").as_deref() == Some("SNIP");
    let mut best: Option<(i32, Pick)> = None;
    for t in list {
        let Some(url) = s(t, "url") else { continue };
        let fmt = t.get("format");
        let protocol = fmt.and_then(|f| s(f, "protocol")).unwrap_or_default().to_ascii_lowercase();
        let mime = fmt.and_then(|f| s(f, "mime_type")).unwrap_or_default().to_ascii_lowercase();
        if protocol.contains("encrypted") || mime.contains("ogg") || mime.contains("opus") {
            continue;
        }
        let aac = mime.contains("mp4") || mime.contains("aac");
        let mp3 = mime.contains("mpeg");
        let mut score = match (protocol.as_str(), aac, mp3) {
            ("hls", true, _) => 30,
            ("hls", _, true) => 20,
            ("progressive", _, true) => 10,
            _ => continue,
        };
        if quality == StreamQuality::Mp3 && mp3 {
            score += 25;
        }
        let preset = s(t, "preset").unwrap_or_default();
        let high = s(t, "quality").as_deref() == Some("hq") || preset.contains("160") || preset.contains("256");
        if (quality == StreamQuality::Low) != high {
            score += 3;
        }
        let snipped = t.get("snipped").and_then(Value::as_bool).unwrap_or(false);
        if snipped {
            score -= 50;
        }
        let pick = Pick { url, hls: protocol == "hls", preview: snipped || policy_snip };
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, pick));
        }
    }
    best.map(|(_, p)| p)
}

/// Треки плейлиста из API сайта: после первых нескольких идут заглушки `{id}`,
/// их подменяем догруженными полными объектами, сохраняя порядок.
pub fn merge_playlist_tracks(raw: &[Value], full: &HashMap<String, Value>) -> Vec<Track> {
    raw.iter()
        .filter_map(|t| {
            if t.get("title").is_some() {
                return track_from(t);
            }
            let id = t.get("id").and_then(Value::as_u64)?.to_string();
            full.get(&id).and_then(track_from)
        })
        .collect()
}

pub fn track_from(v: &Value) -> Option<Track> {
    // элементы ленты/лайков бывают обёрнуты: {"track": {...}} или {"origin": {...}}
    let v = v.get("track").filter(|t| t.is_object()).or_else(|| v.get("origin").filter(|t| t.is_object())).unwrap_or(v);
    if let Some(kind) = v.get("kind").and_then(Value::as_str) {
        if kind != "track" {
            return None;
        }
    }
    let urn = urn_of(v, "tracks")?;
    let user = v.get("user");
    let uploader = user.and_then(|u| s(u, "username")).unwrap_or_default();
    let artist = s(v, "metadata_artist")
        .or_else(|| v.get("publisher_metadata").and_then(|p| s(p, "artist")))
        .unwrap_or_else(|| uploader.clone());
    Some(Track {
        id: format!("sc:{urn}"),
        title: s(v, "title").unwrap_or_else(|| "Без названия".into()),
        artist,
        uploader,
        uploader_urn: user.and_then(|u| urn_of(u, "users")),
        album: v.get("publisher_metadata").and_then(|p| s(p, "album_title")),
        duration_ms: u(v, "duration").or_else(|| u(v, "full_duration")).unwrap_or(0),
        artwork: s(v, "artwork_url").or_else(|| user.and_then(|u| s(u, "avatar_url"))),
        genre: s(v, "genre"),
        permalink: s(v, "permalink_url"),
        plays: u(v, "playback_count"),
        likes: u(v, "favoritings_count").or_else(|| u(v, "likes_count")),
        access: access_of(v),
        urn: Some(urn),
        path: None,
        local_cover: false,
    })
}

pub fn user_from(v: &Value) -> Option<UserInfo> {
    let v = v.get("user").filter(|u| u.is_object() && v.get("kind").is_none()).unwrap_or(v);
    Some(UserInfo {
        urn: urn_of(v, "users")?,
        username: s(v, "username").unwrap_or_else(|| "?".into()),
        avatar: s(v, "avatar_url"),
        followers: u(v, "followers_count"),
        track_count: u(v, "track_count"),
        permalink: s(v, "permalink_url"),
    })
}

pub fn playlist_from(v: &Value) -> Option<PlaylistInfo> {
    let v = v.get("playlist").filter(|t| t.is_object()).unwrap_or(v);
    let user = v.get("user");
    let first_art = v
        .get("tracks")
        .and_then(Value::as_array)
        .and_then(|a| a.iter().find_map(|t| s(t, "artwork_url")));
    Some(PlaylistInfo {
        id: urn_of(v, "playlists")?,
        title: s(v, "title").unwrap_or_else(|| "Плейлист".into()),
        artwork: s(v, "artwork_url").or(first_art).or_else(|| user.and_then(|u| s(u, "avatar_url"))),
        track_count: u(v, "track_count").unwrap_or(0) as u32,
        user: user.and_then(|u| s(u, "username")).unwrap_or_default(),
        user_urn: user.and_then(|u| urn_of(u, "users")),
        permalink: s(v, "permalink_url"),
    })
}

fn collection(v: &Value) -> (Vec<Value>, Option<String>) {
    match v {
        Value::Array(a) => (a.clone(), None),
        _ => (
            v.get("collection").and_then(Value::as_array).cloned().unwrap_or_default(),
            s(v, "next_href"),
        ),
    }
}

fn page<T>(v: &Value, f: impl Fn(&Value) -> Option<T>) -> Page<T> {
    let (items, next) = collection(v);
    Page { items: items.iter().filter_map(f).collect(), next }
}

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    if getrandom::fill(&mut buf).is_err() {
        // крайне маловероятно; подмешиваем время, чтобы не остаться с нулями
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        for (i, b) in buf.iter_mut().enumerate() {
            *b = (t >> ((i % 16) * 8)) as u8 ^ (i as u8).wrapping_mul(31);
        }
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

#[derive(Deserialize)]
struct TokenResp {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

impl SoundCloud {
    pub fn new(http: reqwest::Client) -> Self {
        let mut store: ScStore = read_json(&store_path()).unwrap_or_default();
        if store.redirect_uri.trim().is_empty() {
            store.redirect_uri = DEFAULT_REDIRECT.into();
        }
        Self { http, store: Mutex::new(store), token_lock: tokio::sync::Mutex::new(()), expired: AtomicBool::new(false) }
    }

    fn save(&self) {
        let st = self.store.lock().clone();
        if let Err(e) = write_json(&store_path(), &st) {
            tracing::warn!("не удалось сохранить данные SoundCloud: {e}");
        }
    }

    /// Есть ли чем ходить в SoundCloud: веб-сессия или свои ключи API.
    pub fn configured(&self) -> bool {
        self.web().is_some() || self.has_dev_keys()
    }

    pub fn has_dev_keys(&self) -> bool {
        let st = self.store.lock();
        !st.client_id.trim().is_empty() && !st.client_secret.trim().is_empty()
    }

    /// Токен и client_id веб-сессии, если пользователь вошёл через сайт.
    fn web(&self) -> Option<(String, String)> {
        let st = self.store.lock();
        match (&st.web_token, &st.web_client_id) {
            (Some(t), Some(c)) => Some((t.clone(), c.clone())),
            _ => None,
        }
    }

    pub fn web_mode(&self) -> bool {
        self.web().is_some()
    }

    pub fn banned_token(&self) -> Option<String> {
        self.store.lock().banned_token.clone()
    }

    /// Нужно ли стереть данные окна входа перед открытием (после «Выйти»).
    pub fn take_clear_on_login(&self) -> bool {
        let v = std::mem::take(&mut self.store.lock().clear_on_login);
        if v {
            self.save();
        }
        v
    }

    /// API ответил 401 и сессия сброшена — интерфейс должен показать «Войти».
    pub fn take_expired(&self) -> bool {
        self.expired.swap(false, Ordering::AcqRel)
    }

    /// Завершение входа через сайт: сохраняем сессию и проверяем её запросом профиля.
    pub async fn web_login(&self, token: String, client_id: String) -> Result<UserInfo, String> {
        {
            let mut st = self.store.lock();
            st.web_token = Some(token);
            st.web_client_id = Some(client_id);
            st.banned_token = None;
            st.clear_on_login = false;
        }
        let me = self.v2_get("/me", &[]).await?;
        let user = user_from(&me).ok_or("SoundCloud не вернул профиль")?;
        self.store.lock().user = Some(user.clone());
        self.save();
        Ok(user)
    }

    /// Сессия больше не принимается: забываем её; следующий вход начнётся с чистого профиля.
    fn expire_web(&self, token: &str) {
        let mut st = self.store.lock();
        if st.web_token.as_deref() == Some(token) {
            st.web_token = None;
            st.web_client_id = None;
            st.user = None;
            st.banned_token = Some(token.to_string());
            st.clear_on_login = true;
            drop(st);
            self.save();
            self.expired.store(true, Ordering::Release);
        }
    }

    /// GET к API сайта от имени пользователя.
    async fn v2_get(&self, path_or_url: &str, params: &[(&str, &str)]) -> Result<Value, String> {
        let (token, client_id) = self.web().ok_or("SoundCloud не подключён — нажмите «Войти через SoundCloud»")?;
        let url = v2_url(path_or_url, params, &client_id)?;
        let resp = self
            .http
            .get(url)
            .header(AUTHORIZATION, format!("OAuth {token}"))
            .header(ACCEPT, "application/json; charset=utf-8")
            .send()
            .await
            .map_err(|e| format!("сеть: {e}"))?;
        match resp.status().as_u16() {
            401 => {
                self.expire_web(&token);
                Err("Сессия SoundCloud устарела — войдите снова".into())
            }
            403 => Err("SoundCloud: доступ запрещён".into()),
            404 => Err("SoundCloud: не найдено".into()),
            429 => Err("SoundCloud: слишком много запросов, подождите немного".into()),
            s if !(200..300).contains(&s) => Err(format!("SoundCloud: HTTP {s}")),
            _ => resp.json().await.map_err(|e| format!("ответ SoundCloud: {e}")),
        }
    }

    async fn v2_tracks(&self, path: &str, params: &[(&str, &str)]) -> Result<Page<Track>, String> {
        Ok(page(&self.v2_get(path, params).await?, track_from))
    }

    /// Числовой id текущего пользователя веб-сессии (из профиля или запросом `/me`).
    async fn v2_me_id(&self) -> Result<String, String> {
        if let Some(u) = self.user() {
            return Ok(sc_id(&u.urn).to_string());
        }
        let me = self.v2_get("/me", &[]).await?;
        let user = user_from(&me).ok_or("SoundCloud не вернул профиль")?;
        let id = sc_id(&user.urn).to_string();
        self.store.lock().user = Some(user);
        self.save();
        Ok(id)
    }

    async fn v2_genre(&self, genre: &str) -> Result<Page<Track>, String> {
        let popular = self
            .v2_tracks("/search/tracks", &[("q", "*"), ("filter.genre_or_tag", genre), ("sort", "popular"), ("limit", "40"), ("linked_partitioning", "1")])
            .await;
        match popular {
            Ok(p) if !p.items.is_empty() => Ok(p),
            _ => self.v2_tracks("/search/tracks", &[("q", genre), ("limit", "40"), ("linked_partitioning", "1")]).await,
        }
    }

    async fn v2_playlist_tracks(&self, urn: &str) -> Result<Page<Track>, String> {
        let v = self.v2_get(&format!("/playlists/{}", sc_id(urn)), &[("representation", "full")]).await?;
        let raw: Vec<Value> = v.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
        let missing: Vec<String> = raw
            .iter()
            .filter(|t| t.get("title").is_none())
            .filter_map(|t| t.get("id").and_then(Value::as_u64).map(|n| n.to_string()))
            .collect();
        let mut full = HashMap::new();
        for chunk in missing.chunks(50) {
            let list = self.v2_get("/tracks", &[("ids", &chunk.join(","))]).await?;
            for t in list.as_array().into_iter().flatten() {
                if let Some(n) = t.get("id").and_then(Value::as_u64) {
                    full.insert(n.to_string(), t.clone());
                }
            }
        }
        Ok(Page { items: merge_playlist_tracks(&raw, &full), next: None })
    }

    async fn v2_my_playlists(&self) -> Result<Page<PlaylistInfo>, String> {
        let me = self.v2_me_id().await?;
        let params = [("limit", "50"), ("linked_partitioning", "1")];
        let v = match self.v2_get(&format!("/users/{me}/playlists_without_albums"), &params).await {
            Ok(v) => v,
            Err(_) => self.v2_get(&format!("/users/{me}/playlists"), &params).await?,
        };
        Ok(page(&v, playlist_from))
    }

    /// Поток трека через API сайта: свежий объект трека → transcoding → ссылка на поток.
    async fn v2_stream(&self, t: &Track, quality: StreamQuality) -> Result<(MediaSpec, bool), String> {
        let urn = t.urn.as_deref().ok_or("у трека нет идентификатора SoundCloud")?;
        let v = self.v2_get(&format!("/tracks/{}", sc_id(urn)), &[]).await?;
        if access_of(&v).as_deref() == Some("blocked") {
            return Err("трек недоступен в вашем регионе или заблокирован правообладателем".into());
        }
        let pick = pick_transcoding(&v, quality).ok_or("SoundCloud не отдал поддерживаемый поток (AAC или MP3) для этого трека")?;
        let auth = s(&v, "track_authorization");
        let params: Vec<(&str, &str)> = auth.as_deref().map(|a| vec![("track_authorization", a)]).unwrap_or_default();
        let r = self.v2_get(&pick.url, &params).await?;
        let url = s(&r, "url").ok_or("SoundCloud не вернул ссылку на поток")?;
        let spec = if pick.hls { MediaSpec::Hls { url, auth: None } } else { MediaSpec::Http { url, auth: None } };
        Ok((spec, pick.preview))
    }

    pub fn user(&self) -> Option<UserInfo> {
        self.store.lock().user.clone()
    }

    pub fn credentials(&self) -> (String, String, String) {
        let st = self.store.lock();
        (st.client_id.clone(), st.client_secret.clone(), st.redirect_uri.clone())
    }

    pub fn set_credentials(&self, id: &str, secret: &str, redirect: &str) {
        {
            let mut st = self.store.lock();
            let changed = st.client_id != id.trim() || st.client_secret != secret.trim();
            st.client_id = id.trim().into();
            st.client_secret = secret.trim().into();
            st.redirect_uri = if redirect.trim().is_empty() { DEFAULT_REDIRECT.into() } else { redirect.trim().into() };
            if changed {
                st.app_token = None;
                st.user_token = None;
                st.user = None;
            }
        }
        self.save();
    }

    /// Выход: забываем пользовательские токены. Cookie сессии в окне входа сотрём
    /// при следующем открытии, чтобы можно было войти в другой аккаунт.
    pub fn logout(&self) {
        {
            let mut st = self.store.lock();
            if let Some(t) = st.web_token.take() {
                st.banned_token = Some(t);
                st.clear_on_login = true;
            }
            st.web_client_id = None;
            st.user_token = None;
            st.user = None;
        }
        self.save();
    }

    async fn token_request(&self, form: &[(&str, &str)], basic: Option<(&str, &str)>) -> Result<Token, String> {
        let mut req = self.http.post(TOKEN_URL).header(ACCEPT, "application/json; charset=utf-8").form(form);
        if let Some((id, secret)) = basic {
            req = req.basic_auth(id, Some(secret));
        }
        let resp = req.send().await.map_err(|e| format!("сеть: {e}"))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err("SoundCloud: превышен лимит выдачи токенов, попробуйте позже".into());
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("SoundCloud отклонил ключи (HTTP {status}): {}", body.chars().take(200).collect::<String>()));
        }
        let t: TokenResp = resp.json().await.map_err(|e| format!("ответ токена: {e}"))?;
        Ok(Token { access_token: t.access_token, refresh_token: t.refresh_token, expires_at: now_secs() + t.expires_in.unwrap_or(3600) })
    }

    /// Действующий токен: пользовательский (с обновлением) или токен приложения.
    pub async fn token(&self) -> Result<String, String> {
        let _guard = self.token_lock.lock().await;
        let (user, app, id, secret) = {
            let st = self.store.lock();
            (st.user_token.clone(), st.app_token.clone(), st.client_id.clone(), st.client_secret.clone())
        };
        if let Some(t) = user {
            if t.fresh() {
                return Ok(t.access_token);
            }
            if let Some(rt) = &t.refresh_token {
                match self
                    .token_request(&[("grant_type", "refresh_token"), ("client_id", &id), ("client_secret", &secret), ("refresh_token", rt)], None)
                    .await
                {
                    Ok(nt) => {
                        let token = nt.access_token.clone();
                        self.store.lock().user_token = Some(nt);
                        self.save();
                        return Ok(token);
                    }
                    Err(e) => {
                        tracing::warn!("обновление пользовательского токена не удалось: {e}");
                        let mut st = self.store.lock();
                        st.user_token = None;
                        st.user = None;
                    }
                }
            }
        }
        if let Some(t) = app {
            if t.fresh() {
                return Ok(t.access_token);
            }
        }
        if id.is_empty() || secret.is_empty() {
            return Err("SoundCloud не подключён — нажмите «Войти через SoundCloud»".into());
        }
        let nt = self.token_request(&[("grant_type", "client_credentials")], Some((&id, &secret))).await?;
        let token = nt.access_token.clone();
        self.store.lock().app_token = Some(nt);
        self.save();
        Ok(token)
    }

    fn invalidate(&self, token: &str) {
        let mut st = self.store.lock();
        if st.app_token.as_ref().is_some_and(|t| t.access_token == token) {
            st.app_token = None;
        }
        if let Some(t) = st.user_token.as_mut() {
            if t.access_token == token {
                t.expires_at = 0;
            }
        }
    }

    async fn get(&self, url: &str) -> Result<Value, String> {
        for attempt in 0..2 {
            let token = self.token().await?;
            let resp = self
                .http
                .get(url)
                .header(AUTHORIZATION, format!("OAuth {token}"))
                .header(ACCEPT, "application/json; charset=utf-8")
                .send()
                .await
                .map_err(|e| format!("сеть: {e}"))?;
            let status = resp.status();
            if status.as_u16() == 401 && attempt == 0 {
                self.invalidate(&token);
                continue;
            }
            if status.as_u16() == 429 {
                return Err("SoundCloud: слишком много запросов, подождите немного".into());
            }
            if status.as_u16() == 404 {
                return Err("SoundCloud: не найдено".into());
            }
            if !status.is_success() {
                return Err(format!("SoundCloud: HTTP {status}"));
            }
            return resp.json().await.map_err(|e| format!("ответ SoundCloud: {e}"));
        }
        Err("SoundCloud: требуется повторная авторизация".into())
    }

    fn url(path: &str, params: &[(&str, &str)]) -> String {
        Url::parse_with_params(&format!("{API}{path}"), params).map(|u| u.to_string()).unwrap_or_else(|_| format!("{API}{path}"))
    }

    pub async fn search_tracks(&self, q: &str) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_tracks("/search/tracks", &[("q", q), ("limit", "40"), ("linked_partitioning", "1")]).await;
        }
        let v = self.get(&Self::url("/tracks", &[("q", q), ("limit", "40"), ("linked_partitioning", "true"), ("access", "playable,preview")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn search_playlists(&self, q: &str) -> Result<Page<PlaylistInfo>, String> {
        if self.web_mode() {
            let v = self.v2_get("/search/playlists", &[("q", q), ("limit", "30"), ("linked_partitioning", "1")]).await?;
            return Ok(page(&v, playlist_from));
        }
        let v = self.get(&Self::url("/playlists", &[("q", q), ("limit", "30"), ("linked_partitioning", "true"), ("show_tracks", "false")])).await?;
        Ok(page(&v, playlist_from))
    }

    pub async fn search_users(&self, q: &str) -> Result<Page<UserInfo>, String> {
        if self.web_mode() {
            let v = self.v2_get("/search/users", &[("q", q), ("limit", "30"), ("linked_partitioning", "1")]).await?;
            return Ok(page(&v, user_from));
        }
        let v = self.get(&Self::url("/users", &[("q", q), ("limit", "30"), ("linked_partitioning", "true")])).await?;
        Ok(page(&v, user_from))
    }

    pub async fn genre(&self, genre: &str) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_genre(genre).await;
        }
        let v = self.get(&Self::url("/tracks", &[("genres", genre), ("limit", "40"), ("linked_partitioning", "true"), ("access", "playable")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn related(&self, urn: &str) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_tracks(&format!("/tracks/{}/related", sc_id(urn)), &[("limit", "30"), ("linked_partitioning", "1")]).await;
        }
        let v = self.get(&Self::url(&format!("/tracks/{urn}/related"), &[("limit", "30"), ("linked_partitioning", "true"), ("access", "playable")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn user_tracks(&self, urn: &str) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_tracks(&format!("/users/{}/tracks", sc_id(urn)), &[("limit", "50"), ("linked_partitioning", "1")]).await;
        }
        let v = self.get(&Self::url(&format!("/users/{urn}/tracks"), &[("limit", "50"), ("linked_partitioning", "true"), ("access", "playable,preview")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn user_info(&self, urn: &str) -> Result<UserInfo, String> {
        let v = if self.web_mode() {
            self.v2_get(&format!("/users/{}", sc_id(urn)), &[]).await?
        } else {
            self.get(&Self::url(&format!("/users/{urn}"), &[])).await?
        };
        user_from(&v).ok_or_else(|| "пользователь не найден".into())
    }

    pub async fn playlist_tracks(&self, urn: &str) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_playlist_tracks(urn).await;
        }
        let v = self.get(&Self::url(&format!("/playlists/{urn}/tracks"), &[("linked_partitioning", "true"), ("limit", "100"), ("access", "playable,preview")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn likes(&self) -> Result<Page<Track>, String> {
        if self.web_mode() {
            let me = self.v2_me_id().await?;
            return self.v2_tracks(&format!("/users/{me}/track_likes"), &[("limit", "50"), ("linked_partitioning", "1")]).await;
        }
        let v = self.get(&Self::url("/me/likes/tracks", &[("limit", "50"), ("linked_partitioning", "true"), ("access", "playable,preview")])).await?;
        Ok(page(&v, track_from))
    }

    pub async fn my_playlists(&self) -> Result<Page<PlaylistInfo>, String> {
        if self.web_mode() {
            return self.v2_my_playlists().await;
        }
        let v = self.get(&Self::url("/me/playlists", &[("limit", "50"), ("linked_partitioning", "true"), ("show_tracks", "false")])).await?;
        Ok(page(&v, playlist_from))
    }

    pub async fn feed(&self) -> Result<Page<Track>, String> {
        if self.web_mode() {
            return self.v2_tracks("/stream", &[("limit", "40"), ("linked_partitioning", "1")]).await;
        }
        let v = self.get(&Self::url("/me/feed/tracks", &[("limit", "50"), ("access", "playable,preview")])).await?;
        Ok(page(&v, track_from))
    }

    /// Следующая страница коллекции (`next_href`): ссылки API сайта приходят без client_id.
    async fn next_page(&self, next: &str) -> Result<Value, String> {
        if next.starts_with(API_V2) && self.web_mode() {
            return self.v2_get(next, &[]).await;
        }
        if !next.starts_with(&format!("{API}/")) {
            return Err("неожиданная ссылка пагинации".into());
        }
        self.get(next).await
    }

    pub async fn next_tracks(&self, next: &str) -> Result<Page<Track>, String> {
        Ok(page(&self.next_page(next).await?, track_from))
    }

    pub async fn next_playlists(&self, next: &str) -> Result<Page<PlaylistInfo>, String> {
        Ok(page(&self.next_page(next).await?, playlist_from))
    }

    pub async fn next_users(&self, next: &str) -> Result<Page<UserInfo>, String> {
        Ok(page(&self.next_page(next).await?, user_from))
    }

    /// Выбор потока: AAC HLS (160/96) → MP3 HLS → прогрессивный MP3 → превью.
    /// Возвращает спецификацию и признак «это 30-секундное превью».
    pub async fn stream(&self, t: &Track, quality: StreamQuality) -> Result<(MediaSpec, bool), String> {
        if self.web_mode() {
            return self.v2_stream(t, quality).await;
        }
        let urn = t.urn.as_deref().ok_or("у трека нет идентификатора SoundCloud")?;
        if t.is_blocked() {
            return Err("трек недоступен в вашем регионе или заблокирован правообладателем".into());
        }
        let v = self.get(&Self::url(&format!("/tracks/{urn}/streams"), &[])).await?;
        let token = self.token().await?;
        let auth = Some(format!("OAuth {token}"));
        let order: &[&str] = match quality {
            StreamQuality::High => &["hls_aac_160_url", "hls_aac_96_url", "hls_mp3_128_url"],
            StreamQuality::Low => &["hls_aac_96_url", "hls_aac_160_url", "hls_mp3_128_url"],
            StreamQuality::Mp3 => &["hls_mp3_128_url", "hls_aac_160_url", "hls_aac_96_url"],
        };
        if !t.is_preview() {
            for key in order {
                if let Some(url) = s(&v, key) {
                    return Ok((MediaSpec::Hls { url, auth }, false));
                }
            }
            if let Some(url) = s(&v, "http_mp3_128_url") {
                return Ok((MediaSpec::Http { url, auth }, false));
            }
        }
        if let Some(url) = s(&v, "preview_mp3_128_url") {
            return Ok((MediaSpec::Http { url, auth }, true));
        }
        Err("SoundCloud не отдал ни одного потока для этого трека".into())
    }

    /// Вход пользователя: открываем страницу SoundCloud в браузере и ловим
    /// перенаправление с кодом на локальном порту из Redirect URI.
    pub async fn login(&self) -> Result<UserInfo, String> {
        let (id, secret, redirect) = self.credentials();
        if id.is_empty() || secret.is_empty() {
            return Err("сначала укажите Client ID и Client Secret".into());
        }
        let redirect_url = Url::parse(&redirect).map_err(|_| "Redirect URI должен быть вида http://127.0.0.1:47823/callback")?;
        let host = redirect_url.host_str().unwrap_or_default().to_string();
        if host != "127.0.0.1" && host != "localhost" {
            return Err("Redirect URI должен указывать на 127.0.0.1 или localhost".into());
        }
        let port = redirect_url.port_or_known_default().unwrap_or(80);
        let verifier = random_b64(48);
        let challenge = {
            use sha2::{Digest, Sha256};
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        };
        let state = random_b64(16);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .map_err(|e| format!("порт {port} занят или недоступен: {e}"))?;
        let auth = Url::parse_with_params(
            AUTH_URL,
            &[
                ("client_id", id.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("response_type", "code"),
                ("code_challenge", challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", state.as_str()),
                ("display", "popup"),
            ],
        )
        .map_err(|e| e.to_string())?;
        open::that(auth.as_str()).map_err(|e| format!("не удалось открыть браузер: {e}"))?;

        let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state, redirect_url.path()))
            .await
            .map_err(|_| "время ожидания входа истекло".to_string())??;

        let token = self
            .token_request(
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", &id),
                    ("client_secret", &secret),
                    ("redirect_uri", &redirect),
                    ("code_verifier", &verifier),
                    ("code", &code),
                ],
                None,
            )
            .await?;
        self.store.lock().user_token = Some(token);
        let me = self.get(&format!("{API}/me")).await?;
        let user = user_from(&me).ok_or("не удалось получить профиль")?;
        self.store.lock().user = Some(user.clone());
        self.save();
        Ok(user)
    }
}

const PAGE_OK: &str = "<!doctype html><meta charset=utf-8><title>Nebula</title><body style=\"background:#07051a;color:#ece8ff;font:18px Segoe UI,sans-serif;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><div style=\"font-size:54px\">🪐</div><h2>Готово!</h2><p>Можно закрыть вкладку и вернуться в Nebula.</p></div>";
const PAGE_ERR: &str = "<!doctype html><meta charset=utf-8><title>Nebula</title><body style=\"background:#07051a;color:#ffd6de;font:18px Segoe UI,sans-serif;display:grid;place-items:center;height:100vh;margin:0\"><div style=\"text-align:center\"><h2>Вход не выполнен</h2><p>Вернитесь в Nebula и попробуйте ещё раз.</p></div>";

async fn respond(sock: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let resp = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = sock.write_all(resp.as_bytes()).await;
    let _ = sock.shutdown().await;
}

async fn wait_for_code(listener: &tokio::net::TcpListener, state: &str, path: &str) -> Result<String, String> {
    loop {
        let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 8192];
        let n = match tokio::time::timeout(Duration::from_secs(5), sock.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            _ => continue,
        };
        let req = String::from_utf8_lossy(&buf[..n]);
        let target = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let Ok(url) = Url::parse(&format!("http://127.0.0.1{target}")) else {
            respond(&mut sock, "400 Bad Request", PAGE_ERR).await;
            continue;
        };
        if url.path() != path {
            respond(&mut sock, "404 Not Found", "").await;
            continue;
        }
        let q: std::collections::HashMap<String, String> = url.query_pairs().into_owned().collect();
        if let Some(err) = q.get("error") {
            respond(&mut sock, "200 OK", PAGE_ERR).await;
            return Err(format!("SoundCloud: {err}"));
        }
        if q.get("state").map(String::as_str) != Some(state) {
            respond(&mut sock, "400 Bad Request", PAGE_ERR).await;
            continue;
        }
        if let Some(code) = q.get("code") {
            respond(&mut sock, "200 OK", PAGE_OK).await;
            return Ok(code.clone());
        }
        respond(&mut sock, "400 Bad Request", PAGE_ERR).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track_variants() {
        let v: Value = serde_json::from_str(
            r#"{"kind":"track","urn":"soundcloud:tracks:42","title":"Song","duration":201000,
                "artwork_url":"https://i1.sndcdn.com/artworks-x-large.jpg","access":"playable",
                "metadata_artist":"Real Artist","favoritings_count":7,
                "user":{"urn":"soundcloud:users:9","username":"uploader","avatar_url":"https://a/b-large.jpg"}}"#,
        )
        .unwrap();
        let t = track_from(&v).unwrap();
        assert_eq!(t.id, "sc:soundcloud:tracks:42");
        assert_eq!(t.artist, "Real Artist");
        assert_eq!(t.uploader, "uploader");
        assert_eq!(t.likes, Some(7));
        assert_eq!(t.uploader_urn.as_deref(), Some("soundcloud:users:9"));

        // числовой id, обёртка ленты, без metadata_artist
        let v: Value = serde_json::from_str(r#"{"origin":{"kind":"track","id":5,"title":"X","user":{"id":3,"username":"u"}}}"#).unwrap();
        let t = track_from(&v).unwrap();
        assert_eq!(t.urn.as_deref(), Some("soundcloud:tracks:5"));
        assert_eq!(t.artist, "u");

        // плейлист в ленте — не трек
        let v: Value = serde_json::from_str(r#"{"kind":"playlist","id":1}"#).unwrap();
        assert!(track_from(&v).is_none());
    }

    #[test]
    fn parses_collections() {
        let v: Value = serde_json::from_str(r#"{"collection":[{"kind":"track","id":1,"title":"a"},{"kind":"track","id":2,"title":"b"}],"next_href":"https://api.soundcloud.com/tracks?page=2"}"#).unwrap();
        let p = page(&v, track_from);
        assert_eq!(p.items.len(), 2);
        assert!(p.next.unwrap().contains("page=2"));
    }

    #[test]
    fn pkce_material_is_urlsafe() {
        let v = random_b64(48);
        assert_eq!(v.len(), 64);
        assert!(v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    // ─── API сайта (фикстуры в формате api-v2) ───

    fn v2_track(id: u64, policy: &str) -> Value {
        serde_json::json!({
            "kind": "track", "id": id, "title": format!("Track {id}"), "duration": 180000,
            "policy": policy, "streamable": true,
            "user": {"kind": "user", "id": 77, "username": "artist"},
            "media": {"transcodings": []}
        })
    }

    #[test]
    fn v2_access_from_policy() {
        assert_eq!(track_from(&v2_track(1, "ALLOW")).unwrap().access.as_deref(), Some("playable"));
        assert_eq!(track_from(&v2_track(2, "MONETIZE")).unwrap().access.as_deref(), Some("playable"));
        assert_eq!(track_from(&v2_track(3, "SNIP")).unwrap().access.as_deref(), Some("preview"));
        assert_eq!(track_from(&v2_track(4, "BLOCK")).unwrap().access.as_deref(), Some("blocked"));
        let mut t = v2_track(5, "ALLOW");
        t["streamable"] = Value::Bool(false);
        assert_eq!(track_from(&t).unwrap().access.as_deref(), Some("blocked"));
        // лайк в API сайта обёрнут: {"created_at", "kind": "like", "track": {...}}
        let like = serde_json::json!({"created_at": "2026-01-01", "kind": "like", "track": v2_track(6, "ALLOW")});
        let t = track_from(&like).unwrap();
        assert_eq!(t.urn.as_deref(), Some("soundcloud:tracks:6"));
        assert_eq!(t.uploader_urn.as_deref(), Some("soundcloud:users:77"));
    }

    fn transcodings(list: Value) -> Value {
        serde_json::json!({"policy": "ALLOW", "media": {"transcodings": list}})
    }

    #[test]
    fn picks_supported_stream() {
        let v = transcodings(serde_json::json!([
            {"url": "https://api-v2.soundcloud.com/media/x/drm/stream/hls", "preset": "aac_160k", "format": {"protocol": "ctr-encrypted-hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url": "https://api-v2.soundcloud.com/media/x/opus/stream/hls", "preset": "opus_0_0", "format": {"protocol": "hls", "mime_type": "audio/ogg; codecs=\"opus\""}},
            {"url": "https://api-v2.soundcloud.com/media/x/mp3/stream/hls", "preset": "mp3_1_0", "format": {"protocol": "hls", "mime_type": "audio/mpeg"}},
            {"url": "https://api-v2.soundcloud.com/media/x/prog/stream/progressive", "preset": "mp3_1_0", "format": {"protocol": "progressive", "mime_type": "audio/mpeg"}},
            {"url": "https://api-v2.soundcloud.com/media/x/aac96/stream/hls", "preset": "aac_96k", "quality": "sq", "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}},
            {"url": "https://api-v2.soundcloud.com/media/x/aac160/stream/hls", "preset": "aac_160k", "quality": "hq", "format": {"protocol": "hls", "mime_type": "audio/mp4; codecs=\"mp4a.40.2\""}}
        ]));
        let best = pick_transcoding(&v, StreamQuality::High).unwrap();
        assert!(best.url.ends_with("/aac160/stream/hls") && best.hls && !best.preview, "{best:?}");
        assert!(pick_transcoding(&v, StreamQuality::Low).unwrap().url.ends_with("/aac96/stream/hls"));
        assert!(pick_transcoding(&v, StreamQuality::Mp3).unwrap().url.ends_with("/mp3/stream/hls"));

        // только DRM и Opus — играть нечем
        let only_drm = transcodings(serde_json::json!([
            {"url": "https://a/x", "format": {"protocol": "cbc-encrypted-hls", "mime_type": "audio/mp4"}},
            {"url": "https://a/y", "format": {"protocol": "hls", "mime_type": "audio/ogg; codecs=\"opus\""}}
        ]));
        assert!(pick_transcoding(&only_drm, StreamQuality::High).is_none());

        // превью: обрезанный поток или политика SNIP
        let snip = serde_json::json!({"policy": "SNIP", "media": {"transcodings": [
            {"url": "https://a/p", "snipped": true, "format": {"protocol": "progressive", "mime_type": "audio/mpeg"}}
        ]}});
        let p = pick_transcoding(&snip, StreamQuality::High).unwrap();
        assert!(p.preview && !p.hls);
    }

    #[test]
    fn v2_url_adds_client_id_and_stays_on_host() {
        let u = v2_url("/search/tracks", &[("q", "лофай"), ("limit", "40")], "CID").unwrap();
        let q: HashMap<String, String> = u.query_pairs().into_owned().collect();
        assert_eq!(u.host_str(), Some(API_V2_HOST));
        assert_eq!(q.get("q").map(String::as_str), Some("лофай"));
        assert_eq!(q.get("client_id").map(String::as_str), Some("CID"));

        // next_href приходит без client_id; старый client_id заменяется, параметры сохраняются
        let next = "https://api-v2.soundcloud.com/search/tracks?query_urn=soundcloud%3Asearch%3Aabc&offset=40&q=lofi&client_id=OLD";
        let u = v2_url(next, &[], "NEW").unwrap();
        let q: Vec<(String, String)> = u.query_pairs().into_owned().collect();
        assert!(q.contains(&("offset".into(), "40".into())));
        assert_eq!(q.iter().filter(|(k, _)| k == "client_id").count(), 1);
        assert!(q.contains(&("client_id".into(), "NEW".into())));

        // токен не должен уйти на чужой хост
        assert!(v2_url("https://evil.example.com/me", &[], "CID").is_err());
        assert!(v2_url("http://api-v2.soundcloud.com/me", &[], "CID").is_err());
    }

    #[test]
    fn playlist_stubs_are_hydrated_in_order() {
        let raw = vec![v2_track(1, "ALLOW"), serde_json::json!({"id": 2, "kind": "track", "policy": "ALLOW"}), serde_json::json!({"id": 3, "kind": "track"})];
        let mut full = HashMap::new();
        full.insert("2".to_string(), v2_track(2, "SNIP"));
        let items = merge_playlist_tracks(&raw, &full);
        let ids: Vec<_> = items.iter().map(|t| t.urn.clone().unwrap()).collect();
        assert_eq!(ids, vec!["soundcloud:tracks:1", "soundcloud:tracks:2"]);
        assert_eq!(items[1].access.as_deref(), Some("preview"));
        assert_eq!(sc_id("soundcloud:tracks:42"), "42");
        assert_eq!(sc_id("42"), "42");
    }
}
