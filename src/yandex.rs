//! Public Yandex Music playlist importer.
//!
//! Yandex does not expose a stable public desktop API for playlist export, so this
//! module uses the JSON endpoint currently used by the web player. It intentionally
//! supports public playlists only and keeps the parser tolerant of small response
//! shape changes.

use crate::import::{ImportPlaylist, ImportTrack, Service};
use crate::paths::{paths, read_json, write_json};
use parking_lot::Mutex;
use reqwest::{Client, Url};
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const PLAYLIST_HANDLER: &str = "https://music.yandex.ru/handlers/playlist.jsx";
const PLAYLIST_API: &str = "https://api.music.yandex.ru/playlist";

fn store_path() -> std::path::PathBuf {
    paths().config.join("yandex.json")
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct YandexStore {
    /// OAuth token copied from the user's Yandex Music session.
    token: String,
}

/// Трек в ответе Яндекса (до перевода в общую модель импорта).
#[derive(Clone, Debug, PartialEq)]
struct YandexTrack {
    id: String,
    title: String,
    artists: String,
    duration_ms: u64,
    artwork: Option<String>,
    album: Option<String>,
    album_id: Option<String>,
}

#[derive(Clone, Debug)]
struct PlaylistRef {
    owner: Option<String>,
    kind: Option<String>,
    uuid: Option<String>,
    source_url: String,
}

pub struct YandexMusic {
    http: Client,
    store: Mutex<YandexStore>,
}

impl YandexMusic {
    pub fn new(http: Client) -> Self {
        Self { http, store: Mutex::new(read_json(&store_path()).unwrap_or_default()) }
    }

    pub fn token(&self) -> String {
        self.store.lock().token.clone()
    }

    pub fn set_token(&self, token: &str) {
        let token = token.trim().to_string();
        self.store.lock().token = token;
        let snapshot = self.store.lock().clone();
        if let Err(e) = write_json(&store_path(), &snapshot) {
            tracing::warn!("не удалось сохранить токен Яндекс Музыки: {e}");
        }
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let token = self.store.lock().token.trim().to_string();
        if token.is_empty() {
            req
        } else {
            req.header(AUTHORIZATION, format!("OAuth {token}"))
        }
    }

    /// Fetch a public Yandex playlist from a regular `/users/.../playlists/...`
    /// link, an iframe link, or a modern `/playlists/<uuid>` link.
    pub async fn import_playlist(&self, input: &str) -> Result<ImportPlaylist, String> {
        let reference = parse_playlist_url(input)?;
        let (value, owner, kind) = if let Some(uuid) = reference.uuid.clone() {
            let endpoint = format!("{PLAYLIST_API}/{uuid}");
            let value = self
                .auth(self.http.get(endpoint))
                .header("Accept", "application/json")
                .header("Referer", &reference.source_url)
                .send()
                .await
                .map_err(|e| format!("Яндекс Музыка: сеть: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Яндекс Музыка: плейлист недоступен ({e})"))?
                .json::<Value>()
                .await
                .map_err(|e| format!("Яндекс Музыка: некорректный JSON: {e}"))?;
            (value, String::new(), uuid)
        } else {
            let owner = reference.owner.clone().unwrap_or_default();
            let kind = reference.kind.clone().unwrap_or_default();
            // Современный публичный резолвер принимает пару владелец:kind.
            // Старый handlers endpoint оставлен как fallback для регионов, где
            // новый маршрут ещё не включён.
            let modern_endpoint = format!("{PLAYLIST_API}/{owner}:{kind}");
            if let Ok(resp) = self
                .auth(self.http.get(&modern_endpoint))
                .header("Accept", "application/json")
                .header("Referer", &reference.source_url)
                .send()
                .await
            {
                if resp.status().is_success() {
                    if let Ok(value) = resp.json::<Value>().await {
                        return parse_playlist(value, owner, kind, reference.source_url);
                    }
                }
            }
            let value = self
                .auth(self.http.get(PLAYLIST_HANDLER))
                .query(&[
                    ("owner", owner.as_str()),
                    ("kinds", kind.as_str()),
                    ("light", "false"),
                    ("lang", "ru"),
                    ("external-domain", "music.yandex.ru"),
                    ("overembed", "false"),
                ])
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest")
                .header("Referer", &reference.source_url)
                .send()
                .await
                .map_err(|e| format!("Яндекс Музыка: сеть: {e}"))?
                .error_for_status()
                .map_err(|e| format!("Яндекс Музыка: плейлист недоступен ({e})"))?
                .json::<Value>()
                .await
                .map_err(|e| format!("Яндекс Музыка: публичный endpoint вернул не JSON (возможно, ссылка приватная или регион ограничен): {e}"))?;
            (value, owner, kind)
        };

        parse_playlist(value, owner, kind, reference.source_url)
    }
}

fn parse_playlist_url(input: &str) -> Result<PlaylistRef, String> {
    let raw = input.trim();
    let url = Url::parse(raw).map_err(|_| "Вставьте ссылку на публичный плейлист Яндекс Музыки".to_string())?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !host.starts_with("music.yandex.") {
        return Err("Поддерживаются ссылки music.yandex.ru (только публичные плейлисты)".into());
    }
    let parts: Vec<_> = url.path_segments().map(|p| p.filter(|s| !s.is_empty()).collect()).unwrap_or_default();
    let source_url = url.to_string();
    match parts.as_slice() {
        [users, owner, playlists, kind] if *users == "users" && *playlists == "playlists" => Ok(PlaylistRef {
            owner: Some((*owner).to_string()),
            kind: Some((*kind).to_string()),
            uuid: None,
            source_url,
        }),
        [iframe, playlist, owner, kind] if *iframe == "iframe" && *playlist == "playlist" => Ok(PlaylistRef {
            owner: Some((*owner).to_string()),
            kind: Some((*kind).to_string()),
            uuid: None,
            source_url,
        }),
        [playlists, uuid] if *playlists == "playlists" => Ok(PlaylistRef {
            owner: None,
            kind: None,
            uuid: Some((*uuid).to_string()),
            source_url,
        }),
        _ => Err("Ссылка должна вести на /users/<владелец>/playlists/<номер>".into()),
    }
}

fn parse_playlist(value: Value, owner: String, kind: String, source_url: String) -> Result<ImportPlaylist, String> {
    let playlist = value.get("playlist").or_else(|| value.get("result")).unwrap_or(&value);
    let title = text(playlist, "title").unwrap_or_else(|| "Плейлист Яндекс Музыки".into());
    let owner = if owner.is_empty() {
        text(playlist, "uid").or_else(|| playlist.get("owner").and_then(|v| text(v, "login")).or_else(|| text(playlist, "ownerLogin"))).unwrap_or_else(|| "yandex".into())
    } else {
        owner
    };
    let tracks: Vec<YandexTrack> = playlist
        .get("tracks")
        .and_then(Value::as_array)
        .or_else(|| playlist.get("trackEntries").and_then(Value::as_array))
        .map(|items| items.iter().filter_map(parse_track).collect())
        .unwrap_or_default();
    if tracks.is_empty() {
        return Err("Яндекс Музыка вернула пустой плейлист или скрыла его треки".into());
    }
    let tracks = tracks
        .into_iter()
        .map(|t| ImportTrack {
            // прежний формат id заглушек — уже импортированные плейлисты не ломаются
            key: format!("ym:{owner}:{kind}:{}", t.id),
            permalink: t.album_id.as_ref().map(|album| format!("https://music.yandex.ru/album/{album}/track/{}", t.id)),
            title: t.title,
            artists: t.artists,
            duration_ms: t.duration_ms,
            artwork: t.artwork,
            album: t.album,
        })
        .collect();
    Ok(ImportPlaylist { service: Service::Yandex, title, source_url, tracks })
}

fn parse_track(value: &Value) -> Option<YandexTrack> {
    let v = value.get("track").filter(|v| v.is_object()).unwrap_or(value);
    let id = text(v, "id").or_else(|| text(v, "trackId"))?;
    let title = text(v, "title").unwrap_or_else(|| "Без названия".into());
    let artists = v
        .get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| text(x, "name")).collect::<Vec<_>>().join(", "))
        .filter(|s| !s.is_empty())
        .or_else(|| text(v, "artist"))
        .unwrap_or_else(|| "Неизвестный исполнитель".into());
    let duration_ms = number(v, "durationMs").or_else(|| number(v, "duration_ms")).or_else(|| number(v, "duration").map(|s| s.saturating_mul(1000))).unwrap_or_default();
    let album = v.get("albums").and_then(Value::as_array).and_then(|a| a.first()).and_then(|a| text(a, "title"));
    let album_id = v.get("albums").and_then(Value::as_array).and_then(|a| a.first()).and_then(|a| text(a, "id"));
    Some(YandexTrack { id, title, artists, duration_ms, artwork: artwork(v), album, album_id })
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn number(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|n| n.as_u64().or_else(|| n.as_f64().map(|x| x as u64)))
}

fn artwork(v: &Value) -> Option<String> {
    let raw = text(v, "ogImage").or_else(|| text(v, "coverUri")).or_else(|| v.get("albums").and_then(Value::as_array).and_then(|a| a.first()).and_then(|x| text(x, "coverUri")))?;
    let mut url = raw.replace("%%", "200x200");
    if url.starts_with("//") {
        url = format!("https:{url}");
    } else if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("https://{url}");
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_public_links() {
        let a = parse_playlist_url("https://music.yandex.ru/users/demo/playlists/42").unwrap();
        assert_eq!(a.owner.as_deref(), Some("demo"));
        assert_eq!(a.kind.as_deref(), Some("42"));
        let b = parse_playlist_url("https://music.yandex.ru/iframe/playlist/demo/7").unwrap();
        assert_eq!(b.kind.as_deref(), Some("7"));
        let c = parse_playlist_url("https://music.yandex.ru/playlists/lk.abc").unwrap();
        assert_eq!(c.uuid.as_deref(), Some("lk.abc"));
    }

    #[test]
    fn parses_modern_playlist_shape() {
        let value = json!({
            "result": {
                "title": "Тест",
                "tracks": [{
                    "track": {
                        "id": "42",
                        "title": "Song",
                        "artists": [{"name": "Artist"}],
                        "durationMs": 123000
                    }
                }]
            }
        });
        let playlist = parse_playlist(value, String::new(), "42".into(), "https://music.yandex.ru/playlists/x".into()).unwrap();
        assert_eq!(playlist.title, "Тест");
        assert_eq!(playlist.service, Service::Yandex);
        assert_eq!(playlist.tracks[0].artists, "Artist");
        assert_eq!(playlist.tracks[0].duration_ms, 123000);
        // владелец берётся из ответа, если в ссылке его нет; id заглушки — прежнего формата
        assert!(playlist.tracks[0].key.starts_with("ym:") && playlist.tracks[0].key.ends_with(":42:42"));
    }
}
