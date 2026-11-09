//! Привязка Spotify и перенос из него плейлистов и любимых треков.
//!
//! С февраля 2026 Spotify отдаёт содержимое плейлистов только тому, кто их
//! владеет (или соавтор), а приложения в режиме разработки работают лишь у
//! пользователей, которые вошли в свой аккаунт. Поэтому вход — обязательный:
//! OAuth с PKCE, нужен только Client ID своего приложения на
//! developer.spotify.com (секретный ключ не нужен и не хранится). Владельцу
//! приложения Spotify требует Premium; в приложение можно добавить до 5 человек.
//!
//! Локально (`config/spotify.json`) хранятся Client ID, токен обновления и
//! профиль; токен доступа живёт только в памяти.

use crate::import::{ImportPlaylist, ImportTrack, Service};
use crate::paths::{paths, read_json, write_json};
use crate::soundcloud::{random_b64, wait_for_code};
use base64::Engine as _;
use parking_lot::Mutex;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

const AUTH_URL: &str = "https://accounts.spotify.com/authorize";
const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const API: &str = "https://api.spotify.com/v1";
/// Адрес возврата после входа — его нужно добавить в настройки приложения Spotify.
pub const REDIRECT_URI: &str = "http://127.0.0.1:47824/callback";
const REDIRECT_PORT: u16 = 47824;
const SCOPES: &str = "playlist-read-private playlist-read-collaborative user-library-read";
/// Ограничения на размер импорта (страницы по 50–100 треков).
const MAX_TRACKS: usize = 2000;
const MAX_LIKED: usize = 5000;
pub const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

fn store_path() -> std::path::PathBuf {
    paths().config.join("spotify.json")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpUser {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
}

/// Плейлист из «Моей медиатеки» Spotify.
#[derive(Clone, Debug, PartialEq)]
pub struct SpPlaylist {
    pub id: String,
    pub name: String,
    pub total: u64,
    pub image: Option<String>,
    /// Свой (или совместный) — только такие Spotify отдаёт с треками.
    pub own: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    client_id: String,
    /// Старое поле (ключи client credentials) — больше не используется.
    #[serde(skip_serializing)]
    client_secret: String,
    refresh_token: String,
    user: Option<SpUser>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
}

pub struct Spotify {
    http: Client,
    store: Mutex<Store>,
    token: Mutex<Option<(String, Instant)>>,
}

impl Spotify {
    pub fn new(http: Client) -> Self {
        Self { http, store: Mutex::new(read_json(&store_path()).unwrap_or_default()), token: Mutex::new(None) }
    }

    fn save(&self) {
        let snapshot = self.store.lock().clone();
        if let Err(e) = write_json(&store_path(), &snapshot) {
            tracing::warn!("не удалось сохранить настройки Spotify: {e}");
        }
    }

    pub fn client_id(&self) -> String {
        self.store.lock().client_id.clone()
    }

    pub fn has_client_id(&self) -> bool {
        !self.store.lock().client_id.trim().is_empty()
    }

    pub fn set_client_id(&self, client_id: &str) {
        {
            let mut s = self.store.lock();
            let id = client_id.trim().to_string();
            if s.client_id != id {
                // другое приложение — прежний вход к нему не подходит
                s.refresh_token.clear();
                s.user = None;
                *self.token.lock() = None;
            }
            s.client_id = id;
        }
        self.save();
    }

    /// Привязанный аккаунт Spotify.
    pub fn user(&self) -> Option<SpUser> {
        let s = self.store.lock();
        s.user.clone().filter(|_| !s.refresh_token.is_empty())
    }

    /// Отвязать: забыть вход (Client ID остаётся).
    pub fn logout(&self) {
        {
            let mut s = self.store.lock();
            s.refresh_token.clear();
            s.user = None;
        }
        *self.token.lock() = None;
        self.save();
    }

    async fn token_request(&self, form: &[(&str, &str)]) -> Result<TokenResponse, String> {
        let resp = self
            .http
            .post(TOKEN_URL)
            .form(form)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("Spotify: сеть: {e}"))?;
        let status = resp.status();
        if status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED {
            let body = resp.text().await.unwrap_or_default();
            return Err(if body.contains("invalid_grant") {
                "Вход в Spotify устарел — привяжите аккаунт заново".into()
            } else if body.contains("invalid_client") {
                "Spotify не узнал Client ID — проверьте его в настройках приложения".into()
            } else {
                format!("Spotify отклонил вход: {body}")
            });
        }
        if !status.is_success() {
            return Err(format!("Spotify: не удалось войти ({status})"));
        }
        resp.json().await.map_err(|e| format!("Spotify: некорректный ответ: {e}"))
    }

    fn remember(&self, t: &TokenResponse) {
        // обновляем с запасом в минуту
        let life = Duration::from_secs(t.expires_in.max(120) - 60);
        *self.token.lock() = Some((t.access_token.clone(), Instant::now() + life));
        if let Some(r) = t.refresh_token.as_ref().filter(|r| !r.is_empty()) {
            self.store.lock().refresh_token = r.clone();
        }
    }

    /// Вход в аккаунт Spotify: страница входа открывается в браузере, код
    /// возвращается на локальный порт (PKCE, без секретного ключа).
    pub async fn login(&self) -> Result<SpUser, String> {
        let id = self.client_id();
        if id.trim().is_empty() {
            return Err("Сначала укажите Client ID приложения Spotify".into());
        }
        let verifier = random_b64(48);
        let challenge = {
            use sha2::{Digest, Sha256};
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        };
        let state = random_b64(16);
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", REDIRECT_PORT))
            .await
            .map_err(|e| format!("Порт {REDIRECT_PORT} занят: закройте другое окно входа и попробуйте снова ({e})"))?;
        let auth = Url::parse_with_params(
            AUTH_URL,
            &[
                ("client_id", id.as_str()),
                ("response_type", "code"),
                ("redirect_uri", REDIRECT_URI),
                ("code_challenge_method", "S256"),
                ("code_challenge", challenge.as_str()),
                ("scope", SCOPES),
                ("state", state.as_str()),
            ],
        )
        .map_err(|e| e.to_string())?;
        open::that(auth.as_str()).map_err(|e| format!("Не удалось открыть браузер: {e}"))?;

        let code = tokio::time::timeout(Duration::from_secs(300), wait_for_code(&listener, &state, "/callback", "Spotify"))
            .await
            .map_err(|_| "Время ожидания входа истекло".to_string())??;
        let t = self
            .token_request(&[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT_URI),
                ("client_id", &id),
                ("code_verifier", &verifier),
            ])
            .await?;
        self.remember(&t);
        let me = self.get_json(&format!("{API}/me")).await?;
        let user = SpUser {
            id: str_of(&me, "id").unwrap_or_default(),
            name: str_of(&me, "display_name").filter(|s| !s.trim().is_empty()).or_else(|| str_of(&me, "id")).unwrap_or_else(|| "Spotify".into()),
            avatar: me.get("images").and_then(|i| best_image(i, 120)),
        };
        self.store.lock().user = Some(user.clone());
        self.save();
        Ok(user)
    }

    async fn access_token(&self, fresh: bool) -> Result<String, String> {
        if !fresh {
            if let Some((t, until)) = self.token.lock().clone() {
                if Instant::now() < until {
                    return Ok(t);
                }
            }
        }
        let (id, refresh) = {
            let s = self.store.lock();
            (s.client_id.trim().to_string(), s.refresh_token.clone())
        };
        if refresh.is_empty() {
            return Err("Привяжите Spotify в настройках (вкладка SoundCloud)".into());
        }
        match self.token_request(&[("grant_type", "refresh_token"), ("refresh_token", &refresh), ("client_id", &id)]).await {
            Ok(t) => {
                self.remember(&t);
                self.save();
                Ok(t.access_token)
            }
            Err(e) => {
                if e.contains("устарел") {
                    self.logout();
                }
                Err(e)
            }
        }
    }

    async fn get_json(&self, url: &str) -> Result<Value, String> {
        let mut fresh = false;
        loop {
            let token = self.access_token(fresh).await?;
            let resp = self
                .http
                .get(url)
                .bearer_auth(&token)
                .timeout(Duration::from_secs(20))
                .send()
                .await
                .map_err(|e| format!("Spotify: сеть: {e}"))?;
            match resp.status() {
                s if s.is_success() => return resp.json::<Value>().await.map_err(|e| format!("Spotify: некорректный ответ: {e}")),
                // токен истёк раньше срока — один раз берём новый
                StatusCode::UNAUTHORIZED if !fresh => fresh = true,
                StatusCode::FORBIDDEN => {
                    return Err("Spotify не пустил: ваш аккаунт должен быть добавлен в приложение (Dashboard → User Management), а у владельца приложения — Premium".into())
                }
                StatusCode::NOT_FOUND => return Err(NOT_FOUND.into()),
                StatusCode::TOO_MANY_REQUESTS => return Err("Spotify просит подождать: слишком много запросов. Повторите через минуту".into()),
                s => return Err(format!("Spotify ответил {s}")),
            }
        }
    }

    /// Плейлисты из медиатеки пользователя (свои, совместные и сохранённые).
    pub async fn my_playlists(&self) -> Result<Vec<SpPlaylist>, String> {
        let me = self.user().map(|u| u.id).unwrap_or_default();
        let mut url = format!("{API}/me/playlists?limit=50");
        let mut out = Vec::new();
        loop {
            let page = self.get_json(&url).await?;
            for p in page.get("items").and_then(Value::as_array).into_iter().flatten() {
                let Some(id) = str_of(p, "id") else { continue };
                // после изменений 2026 года поле «tracks» стало «items»
                let counts = p.get("items").filter(|v| v.is_object()).or_else(|| p.get("tracks").filter(|v| v.is_object()));
                let owner = p.get("owner").and_then(|o| str_of(o, "id")).unwrap_or_default();
                let collaborative = p.get("collaborative").and_then(Value::as_bool).unwrap_or(false);
                out.push(SpPlaylist {
                    id,
                    name: str_of(p, "name").filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "Без названия".into()),
                    total: counts.and_then(|c| c.get("total")).and_then(Value::as_u64).unwrap_or(0),
                    image: p.get("images").and_then(|i| best_image(i, 120)),
                    own: owner == me || collaborative,
                });
            }
            match page.get("next").and_then(Value::as_str) {
                Some(next) if out.len() < 1000 => url = next.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    /// Любимые треки («Любимые треки» / Liked Songs).
    pub async fn liked(&self) -> Result<ImportPlaylist, String> {
        let mut url = format!("{API}/me/tracks?limit=50");
        let mut tracks = Vec::new();
        loop {
            let page = self.get_json(&url).await?;
            tracks.extend(parse_items(&page, "liked"));
            match page.get("next").and_then(Value::as_str) {
                Some(next) if tracks.len() < MAX_LIKED => url = next.to_string(),
                _ => break,
            }
        }
        if tracks.is_empty() {
            return Err("В «Любимых треках» Spotify пусто".into());
        }
        Ok(ImportPlaylist { service: Service::Spotify, title: "Любимые треки".into(), source_url: "https://open.spotify.com/collection/tracks".into(), tracks })
    }

    /// Загрузить свой плейлист по ссылке `open.spotify.com/playlist/…`,
    /// URI `spotify:playlist:…` или голому id.
    pub async fn import_playlist(&self, input: &str) -> Result<ImportPlaylist, String> {
        let id = parse_playlist_id(input)?;
        let meta = self.get_json(&format!("{API}/playlists/{id}?fields=name")).await?;
        let title = meta.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Плейлист Spotify").to_string();

        let mut url = format!("{API}/playlists/{id}/items?limit=100&additional_types=track");
        let mut tracks = Vec::new();
        loop {
            let page = self.get_json(&url).await?;
            tracks.extend(parse_items(&page, &id));
            match page.get("next").and_then(Value::as_str) {
                Some(next) if tracks.len() < MAX_TRACKS => url = next.to_string(),
                _ => break,
            }
        }
        if tracks.is_empty() {
            return Err("В плейлисте нет треков, которые можно перенести (подкасты и локальные файлы Spotify не переносятся)".into());
        }
        Ok(ImportPlaylist { service: Service::Spotify, title, source_url: format!("https://open.spotify.com/playlist/{id}"), tracks })
    }
}

const NOT_FOUND: &str = "Плейлист не найден. Spotify отдаёт треки только из ваших собственных и совместных плейлистов — чужой плейлист сначала скопируйте к себе";

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn is_id(s: &str) -> bool {
    s.len() == 22 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// id плейлиста из ссылки, URI или самого id.
pub fn parse_playlist_id(input: &str) -> Result<String, String> {
    let s = input.trim();
    let bad = || "Вставьте ссылку на плейлист Spotify: open.spotify.com/playlist/…".to_string();
    if let Some(rest) = s.strip_prefix("spotify:playlist:") {
        return if is_id(rest) { Ok(rest.to_string()) } else { Err(bad()) };
    }
    if is_id(s) {
        return Ok(s.to_string());
    }
    let url = Url::parse(s).map_err(|_| bad())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !matches!(host.as_str(), "open.spotify.com" | "play.spotify.com") {
        return Err(bad());
    }
    // /playlist/<id>, /intl-ru/playlist/<id>, /user/<имя>/playlist/<id>, /embed/playlist/<id>
    let parts: Vec<&str> = url.path_segments().map(|p| p.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    parts
        .windows(2)
        .find(|w| w[0] == "playlist" && is_id(w[1]))
        .map(|w| w[1].to_string())
        .ok_or_else(|| {
            if parts.contains(&"album") {
                "Это альбом, а не плейлист: вставьте ссылку на плейлист".to_string()
            } else {
                bad()
            }
        })
}

/// Треки страницы: у плейлистов трек теперь в поле `item` (раньше `track`),
/// у «Любимых треков» — по-прежнему в `track`.
fn parse_items(page: &Value, source: &str) -> Vec<ImportTrack> {
    let Some(items) = page.get("items").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let t = item.get("item").filter(|t| t.is_object()).or_else(|| item.get("track").filter(|t| t.is_object()))?;
            if t.get("is_local").and_then(Value::as_bool).unwrap_or(false) {
                return None;
            }
            if t.get("type").and_then(Value::as_str).is_some_and(|ty| ty != "track") {
                return None;
            }
            let id = t.get("id").and_then(Value::as_str)?.to_string();
            let title = t.get("name").and_then(Value::as_str).unwrap_or("Без названия").to_string();
            let artists = t
                .get("artists")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(", "))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Неизвестный исполнитель".into());
            let album = t.get("album");
            Some(ImportTrack {
                key: format!("sp:{source}:{id}"),
                permalink: Some(format!("https://open.spotify.com/track/{id}")),
                title,
                artists,
                duration_ms: t.get("duration_ms").and_then(Value::as_u64).unwrap_or_default(),
                artwork: album.and_then(|a| a.get("images")).and_then(|i| best_image(i, 300)),
                album: album.and_then(|a| a.get("name")).and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

/// Картинка ближе всего к нужной ширине (Spotify отдаёт 640, 300 и 64).
fn best_image(images: &Value, want: u64) -> Option<String> {
    images
        .as_array()?
        .iter()
        .filter_map(|i| Some((i.get("url")?.as_str()?, i.get("width").and_then(Value::as_u64).unwrap_or(0))))
        .min_by_key(|(_, w)| w.abs_diff(want))
        .map(|(u, _)| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ID: &str = "37i9dQZF1DXcBWIGoYBM5M";

    #[test]
    fn accepts_links_uris_and_ids() {
        for input in [
            format!("https://open.spotify.com/playlist/{ID}?si=abc123"),
            format!("https://open.spotify.com/intl-ru/playlist/{ID}"),
            format!("https://open.spotify.com/user/someone/playlist/{ID}"),
            format!("https://open.spotify.com/embed/playlist/{ID}?utm_source=x"),
            format!("spotify:playlist:{ID}"),
            format!("  {ID}  "),
        ] {
            assert_eq!(parse_playlist_id(&input).as_deref(), Ok(ID), "{input}");
        }
        assert!(parse_playlist_id("https://example.com/playlist/37i9dQZF1DXcBWIGoYBM5M").is_err());
        assert!(parse_playlist_id(&format!("https://open.spotify.com/album/{ID}")).unwrap_err().contains("альбом"));
        assert!(parse_playlist_id("hello").is_err());
    }

    #[test]
    fn parses_new_item_and_old_track_fields() {
        let page = json!({
            "items": [
                // новый формат плейлиста: трек в поле item
                { "item": { "id": "t1", "name": "Song", "duration_ms": 201000, "type": "track", "is_local": false,
                    "artists": [{ "name": "A" }, { "name": "B" }],
                    "album": { "name": "Album", "images": [
                        { "url": "https://i.scdn.co/640", "width": 640 },
                        { "url": "https://i.scdn.co/300", "width": 300 },
                        { "url": "https://i.scdn.co/64", "width": 64 }
                    ] } } },
                // «Любимые треки»: трек в поле track
                { "added_at": "2026-01-01T00:00:00Z", "track": { "id": "t2", "name": "Liked", "type": "track", "artists": [{ "name": "C" }] } },
                { "item": { "id": "e1", "name": "Podcast", "type": "episode" } },
                { "item": { "id": null, "name": "Local", "type": "track", "is_local": true } },
                { "item": true }
            ],
            "next": null
        });
        let tracks = parse_items(&page, "pl");
        assert_eq!(tracks.len(), 2);
        let t = &tracks[0];
        assert_eq!((t.title.as_str(), t.artists.as_str(), t.duration_ms), ("Song", "A, B", 201000));
        assert_eq!(t.album.as_deref(), Some("Album"));
        assert_eq!(t.artwork.as_deref(), Some("https://i.scdn.co/300"));
        assert_eq!(t.key, "sp:pl:t1");
        assert_eq!(tracks[1].title, "Liked");
    }
}
