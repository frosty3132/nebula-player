//! Импорт публичных плейлистов Spotify через официальный Web API.
//!
//! Spotify отдаёт плейлисты только приложениям с ключом, поэтому пользователь
//! один раз создаёт своё приложение на developer.spotify.com (бесплатно) и
//! вставляет Client ID и Client Secret. Ключи хранятся только локально
//! (`config/spotify.json`). Токен — client credentials: вход в аккаунт не нужен,
//! доступны публичные плейлисты пользователей.

use crate::import::{ImportPlaylist, ImportTrack, Service};
use crate::paths::{paths, read_json, write_json};
use parking_lot::Mutex;
use reqwest::{Client, StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};

const TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const API: &str = "https://api.spotify.com/v1";
/// Ограничение на размер импортируемого плейлиста (страницы по 100 треков).
const MAX_TRACKS: usize = 2000;
pub const DASHBOARD_URL: &str = "https://developer.spotify.com/dashboard";

fn store_path() -> std::path::PathBuf {
    paths().config.join("spotify.json")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    client_id: String,
    client_secret: String,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
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

    /// Заданы ли ключи приложения.
    pub fn configured(&self) -> bool {
        let s = self.store.lock();
        !s.client_id.trim().is_empty() && !s.client_secret.trim().is_empty()
    }

    pub fn client_id(&self) -> String {
        self.store.lock().client_id.clone()
    }

    pub fn set_credentials(&self, client_id: &str, client_secret: &str) {
        let snapshot = {
            let mut s = self.store.lock();
            s.client_id = client_id.trim().to_string();
            s.client_secret = client_secret.trim().to_string();
            s.clone()
        };
        *self.token.lock() = None;
        if let Err(e) = write_json(&store_path(), &snapshot) {
            tracing::warn!("не удалось сохранить ключи Spotify: {e}");
        }
    }

    async fn access_token(&self, fresh: bool) -> Result<String, String> {
        if !fresh {
            if let Some((t, until)) = self.token.lock().clone() {
                if Instant::now() < until {
                    return Ok(t);
                }
            }
        }
        let (id, secret) = {
            let s = self.store.lock();
            (s.client_id.trim().to_string(), s.client_secret.trim().to_string())
        };
        if id.is_empty() || secret.is_empty() {
            return Err("Укажите Client ID и Client Secret приложения Spotify".into());
        }
        let resp = self
            .http
            .post(TOKEN_URL)
            .basic_auth(id, Some(secret))
            .form(&[("grant_type", "client_credentials")])
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| format!("Spotify: сеть: {e}"))?;
        if matches!(resp.status(), StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED) {
            return Err("Spotify не принял Client ID / Client Secret — проверьте ключи приложения".into());
        }
        if !resp.status().is_success() {
            return Err(format!("Spotify: не удалось получить доступ ({})", resp.status()));
        }
        let t: TokenResponse = resp.json().await.map_err(|e| format!("Spotify: некорректный ответ: {e}"))?;
        // обновляем с запасом в минуту
        let life = Duration::from_secs(t.expires_in.max(120) - 60);
        *self.token.lock() = Some((t.access_token.clone(), Instant::now() + life));
        Ok(t.access_token)
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
                StatusCode::NOT_FOUND => return Err(NOT_FOUND.into()),
                StatusCode::TOO_MANY_REQUESTS => return Err("Spotify просит подождать: слишком много запросов. Повторите через минуту".into()),
                s => return Err(format!("Spotify ответил {s}")),
            }
        }
    }

    /// Загрузить публичный плейлист по ссылке `open.spotify.com/playlist/…`,
    /// URI `spotify:playlist:…` или голому id.
    pub async fn import_playlist(&self, input: &str) -> Result<ImportPlaylist, String> {
        let id = parse_playlist_id(input)?;
        let meta = self.get_json(&format!("{API}/playlists/{id}?fields=name")).await?;
        let title = meta.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or("Плейлист Spotify").to_string();

        let fields = "next,items(track(id,name,duration_ms,type,is_local,artists(name),album(name,images)))";
        let mut url = format!("{API}/playlists/{id}/tracks?limit=100&additional_types=track&fields={fields}");
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

const NOT_FOUND: &str = "Плейлист не найден или закрыт. Подборки, которые составляет сам Spotify (Daily Mix, «Открытия недели», редакционные), API не отдаёт — скопируйте треки в свой публичный плейлист и импортируйте его";

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

fn parse_items(page: &Value, playlist_id: &str) -> Vec<ImportTrack> {
    let Some(items) = page.get("items").and_then(Value::as_array) else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| {
            let t = item.get("track").filter(|t| t.is_object())?;
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
                key: format!("sp:{playlist_id}:{id}"),
                permalink: Some(format!("https://open.spotify.com/track/{id}")),
                title,
                artists,
                duration_ms: t.get("duration_ms").and_then(Value::as_u64).unwrap_or_default(),
                artwork: album.and_then(cover),
                album: album.and_then(|a| a.get("name")).and_then(Value::as_str).map(str::to_string),
            })
        })
        .collect()
}

/// Обложка альбома ~300 px (Spotify отдаёт 640, 300 и 64).
fn cover(album: &Value) -> Option<String> {
    let images = album.get("images")?.as_array()?;
    images
        .iter()
        .filter_map(|i| Some((i.get("url")?.as_str()?, i.get("width").and_then(Value::as_u64).unwrap_or(0))))
        .min_by_key(|(_, w)| w.abs_diff(300))
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
    fn parses_tracks_and_skips_episodes_and_local_files() {
        let page = json!({
            "items": [
                { "track": { "id": "t1", "name": "Song", "duration_ms": 201000, "type": "track", "is_local": false,
                    "artists": [{ "name": "A" }, { "name": "B" }],
                    "album": { "name": "Album", "images": [
                        { "url": "https://i.scdn.co/640", "width": 640 },
                        { "url": "https://i.scdn.co/300", "width": 300 },
                        { "url": "https://i.scdn.co/64", "width": 64 }
                    ] } } },
                { "track": { "id": "e1", "name": "Podcast", "type": "episode" } },
                { "track": { "id": null, "name": "Local", "type": "track", "is_local": true } },
                { "track": null }
            ],
            "next": null
        });
        let tracks = parse_items(&page, "pl");
        assert_eq!(tracks.len(), 1);
        let t = &tracks[0];
        assert_eq!(t.title, "Song");
        assert_eq!(t.artists, "A, B");
        assert_eq!(t.duration_ms, 201000);
        assert_eq!(t.album.as_deref(), Some("Album"));
        assert_eq!(t.artwork.as_deref(), Some("https://i.scdn.co/300"));
        assert_eq!(t.key, "sp:pl:t1");
        assert_eq!(t.permalink.as_deref(), Some("https://open.spotify.com/track/t1"));
    }
}
