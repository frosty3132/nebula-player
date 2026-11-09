//! Импорт плейлистов из других сервисов (Яндекс Музыка, Spotify).
//!
//! Сервисы отдают только метаданные (название, исполнитель, длительность), поэтому
//! каждый трек ищется в локальной библиотеке, а затем в SoundCloud. Ненайденные
//! треки остаются в плейлисте заглушками — порядок сохраняется, и их можно
//! заменить позже.

use crate::model::{Page, Track};
use crate::soundcloud::SoundCloud;
use futures_util::stream::{self, StreamExt};

/// Сколько ненайденных локально треков ищем в SoundCloud за один импорт.
const MAX_SC_MATCHES: usize = 150;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Service {
    Yandex,
    Spotify,
}

impl Service {
    pub fn label(self) -> &'static str {
        match self {
            Service::Yandex => "Яндекс Музыка",
            Service::Spotify => "Spotify",
        }
    }

    /// Приставка к названию импортированного плейлиста.
    pub fn short(self) -> &'static str {
        match self {
            Service::Yandex => "Яндекс",
            Service::Spotify => "Spotify",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportTrack {
    /// Постоянный id заглушки, если трек не найден (`ym:…`, `sp:…`).
    pub key: String,
    pub title: String,
    pub artists: String,
    pub duration_ms: u64,
    pub artwork: Option<String>,
    pub album: Option<String>,
    /// Ссылка на трек в исходном сервисе.
    pub permalink: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImportPlaylist {
    pub service: Service,
    pub title: String,
    pub source_url: String,
    pub tracks: Vec<ImportTrack>,
}

/// Сопоставить треки плейлиста с локальной библиотекой и SoundCloud.
/// Результат — в исходном порядке; ненайденные — заглушки (`access: unavailable`).
pub async fn match_tracks(playlist: &ImportPlaylist, local: &[Track], sc: Option<&SoundCloud>) -> Vec<Track> {
    match_tracks_with(playlist, local, sc, MAX_SC_MATCHES).await
}

/// То же с другим пределом поиска в SoundCloud (для больших списков вроде любимых треков).
pub async fn match_tracks_with(playlist: &ImportPlaylist, local: &[Track], sc: Option<&SoundCloud>, max_sc: usize) -> Vec<Track> {
    let mut result = vec![None; playlist.tracks.len()];
    let mut pending = Vec::new();
    for (index, source) in playlist.tracks.iter().enumerate() {
        let title = normalize(&source.title);
        let artist = normalize(&source.artists);
        if let Some(found) = local.iter().find(|t| normalize(&t.title) == title && artist_matches(&t.artist, &artist)) {
            result[index] = Some(found.clone());
            continue;
        }
        pending.push((index, source.clone()));
    }

    if let Some(sc) = sc {
        let resolved = stream::iter(pending.iter().take(max_sc).cloned().map(|(index, source)| async move {
            let found = search_soundcloud(sc, &source).await;
            (index, found)
        }))
        .buffered(4)
        .collect::<Vec<_>>()
        .await;
        for (index, found) in resolved {
            result[index] = found;
        }
    }

    for (index, source) in pending {
        if result[index].is_none() {
            result[index] = Some(placeholder(&source));
        }
    }
    result.into_iter().flatten().collect()
}

/// Найден ли трек (а не оставлен заглушкой).
pub fn is_matched(t: &Track) -> bool {
    t.access.as_deref() != Some("unavailable")
}

async fn search_soundcloud(sc: &SoundCloud, source: &ImportTrack) -> Option<Track> {
    let title = normalize(&source.title);
    let artist = normalize(&source.artists);
    let queries = [format!("{} {}", source.artists, source.title), source.title.clone()];
    for query in queries {
        if let Ok(Page { items, .. }) = sc.search_tracks(&query).await {
            if let Some(found) = pick_soundcloud_match(&items, &title, &artist, source.duration_ms) {
                return Some(found.clone());
            }
        }
    }
    None
}

pub(crate) fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| if c == '&' { ' ' } else { c })
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .filter(|word| !matches!(*word, "official" | "audio" | "video" | "lyrics" | "hd"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Лучший кандидат из выдачи SoundCloud: совпадение названия и исполнителя,
/// плюс близкая длительность (отсекает ремиксы, нарезки и часовые «loop»-версии).
fn pick_soundcloud_match<'a>(items: &'a [Track], title: &str, artist: &str, duration_ms: u64) -> Option<&'a Track> {
    let best = items.iter().max_by_key(|t| {
        let mut score = 0i32;
        let candidate_title = normalize(&t.title);
        if candidate_title == title {
            score += 5;
        } else if candidate_title.contains(title) || title.contains(&candidate_title) {
            score += 2;
        }
        if artist_matches(&t.artist, artist) || artist_matches(&t.uploader, artist) {
            score += 4;
        }
        if duration_ms > 0 && t.duration_ms > 0 {
            let diff = duration_ms.abs_diff(t.duration_ms);
            if diff <= 5_000 {
                score += 2;
            } else if diff > 45_000 {
                score -= 2;
            }
        }
        if t.is_blocked() {
            score -= 5;
        }
        score
    })?;
    let title_match = normalize(&best.title) == title;
    let artist_match = artist_matches(&best.artist, artist) || artist_matches(&best.uploader, artist);
    (title_match || artist_match).then_some(best)
}

fn artist_matches(candidate: &str, requested: &str) -> bool {
    let candidate = normalize(candidate);
    let requested_parts = requested.replace(" feat ", ",").replace(" ft ", ",").replace(" featuring ", ",");
    normalize(requested) == candidate || requested_parts.split(',').any(|part| normalize(part) == candidate)
}

fn placeholder(source: &ImportTrack) -> Track {
    Track {
        id: source.key.clone(),
        title: source.title.clone(),
        artist: source.artists.clone(),
        uploader: source.artists.clone(),
        album: source.album.clone(),
        duration_ms: source.duration_ms,
        artwork: source.artwork.clone(),
        permalink: source.permalink.clone(),
        access: Some("unavailable".into()),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(title: &str, artist: &str, duration_ms: u64) -> Track {
        Track { id: format!("sc:{title}"), title: title.into(), artist: artist.into(), uploader: artist.into(), duration_ms, ..Default::default() }
    }

    #[test]
    fn normalizes_names() {
        assert_eq!(normalize("  MØ - Night!  "), "mø night");
        assert_eq!(normalize("Song (Official Audio)"), "song");
    }

    #[test]
    fn prefers_same_length_over_extended_versions() {
        let items = [sc("Song", "Artist", 3_600_000), sc("Song", "Artist", 201_000)];
        let best = pick_soundcloud_match(&items, "song", "artist", 200_000).unwrap();
        assert_eq!(best.duration_ms, 201_000);
        // без совпадения по названию и исполнителю ничего не подставляем
        let other = [sc("Different", "Someone", 200_000)];
        assert!(pick_soundcloud_match(&other, "song", "artist", 200_000).is_none());
    }

    #[test]
    fn unmatched_tracks_become_placeholders() {
        let t = ImportTrack {
            key: "sp:pl:1".into(),
            title: "Song".into(),
            artists: "Artist".into(),
            duration_ms: 1000,
            artwork: None,
            album: None,
            permalink: Some("https://open.spotify.com/track/1".into()),
        };
        let p = placeholder(&t);
        assert_eq!(p.id, "sp:pl:1");
        assert!(!is_matched(&p));
    }
}
