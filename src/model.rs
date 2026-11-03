//! Общие модели данных: трек, плейлист, пользователь, страница выдачи.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Track {
    /// `sc:<urn>` для SoundCloud, `file:<путь>` для локальных файлов.
    pub id: String,
    pub title: String,
    pub artist: String,
    pub uploader: String,
    pub uploader_urn: Option<String>,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub artwork: Option<String>,
    pub genre: Option<String>,
    pub permalink: Option<String>,
    pub plays: Option<u64>,
    pub likes: Option<u64>,
    /// `playable` / `preview` / `blocked`
    pub access: Option<String>,
    pub urn: Option<String>,
    pub path: Option<String>,
    pub local_cover: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtSize {
    Small,
    Medium,
    Large,
}

impl Track {
    pub fn is_local(&self) -> bool {
        self.path.is_some()
    }

    pub fn is_preview(&self) -> bool {
        self.access.as_deref() == Some("preview")
    }

    pub fn is_blocked(&self) -> bool {
        self.access.as_deref() == Some("blocked")
    }

    /// URL обложки для `<img src>` — всегда через наш кэширующий обработчик `/media`.
    pub fn art(&self, size: ArtSize) -> Option<String> {
        if let Some(p) = &self.path {
            return self.local_cover.then(|| format!("/media/cover?p={}", urlencoding::encode(p)));
        }
        self.artwork.as_deref().map(|u| image_url(&sc_art_size(u, size)))
    }
}

/// SoundCloud отдаёт `...-large.jpg` (100×100); подставляем нужный размер.
pub fn sc_art_size(url: &str, size: ArtSize) -> String {
    let suffix = match size {
        ArtSize::Small => "-large",
        ArtSize::Medium => "-t300x300",
        ArtSize::Large => "-t500x500",
    };
    for known in ["-large", "-t500x500", "-t300x300", "-crop", "-original", "-t67x67", "-small", "-badge", "-tiny", "-mini"] {
        if let Some(pos) = url.rfind(known) {
            let mut s = String::with_capacity(url.len() + 8);
            s.push_str(&url[..pos]);
            s.push_str(suffix);
            s.push_str(&url[pos + known.len()..]);
            return s;
        }
    }
    url.to_string()
}

pub fn image_url(remote: &str) -> String {
    format!("/media/img?u={}", urlencoding::encode(remote))
}

pub fn file_url(path: &str) -> String {
    format!("/media/file?p={}", urlencoding::encode(path))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub artwork: Option<String>,
    pub track_count: u32,
    pub user: String,
    pub user_urn: Option<String>,
    pub permalink: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct UserInfo {
    pub urn: String,
    pub username: String,
    pub avatar: Option<String>,
    pub followers: Option<u64>,
    pub track_count: Option<u64>,
    pub permalink: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Self { items: Vec::new(), next: None }
    }
}

pub fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Человеческий формат суммарного времени прослушивания для карточек статистики.
pub fn fmt_listening(ms: u64) -> String {
    let minutes = ms / 60_000;
    let hours = minutes / 60;
    let mins = minutes % 60;
    if hours > 0 {
        format!("{hours} ч {mins:02} мин")
    } else if mins > 0 {
        format!("{mins} мин")
    } else {
        format!("{} с", ms / 1000)
    }
}

pub fn fmt_count(n: u64) -> String {
    let short = |v: f64| {
        let s = format!("{v:.1}");
        s.strip_suffix(".0").map(str::to_string).unwrap_or(s).replace('.', ",")
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{} тыс.", short(n as f64 / 1e3)),
        _ => format!("{} млн", short(n as f64 / 1e6)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artwork_resize() {
        assert_eq!(
            sc_art_size("https://i1.sndcdn.com/artworks-000-abc-large.jpg", ArtSize::Large),
            "https://i1.sndcdn.com/artworks-000-abc-t500x500.jpg"
        );
        assert_eq!(sc_art_size("https://x/y.png", ArtSize::Large), "https://x/y.png");
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_time(225_000), "3:45");
        assert_eq!(fmt_time(3_725_000), "1:02:05");
        assert_eq!(fmt_count(1_500), "1,5 тыс.");
        assert_eq!(fmt_count(2_000_000), "2 млн");
        assert_eq!(fmt_listening(3_725_000), "1 ч 02 мин");
        assert_eq!(fmt_listening(45_000), "45 с");
    }
}
