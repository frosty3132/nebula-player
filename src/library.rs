//! Локальная библиотека: избранное, история, свои плейлисты, сохранённая сессия
//! и индекс музыкальных файлов с компьютера (теги и обложки через lofty).

use crate::model::Track;
use crate::paths::{hash_key, now_secs, paths, read_json, write_json};
use lofty::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const AUDIO_EXT: &[&str] = &["mp3", "flac", "wav", "ogg", "oga", "m4a", "mp4", "aac", "aif", "aiff", "wv"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LocalPlaylist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<Track>,
    pub created: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SavedSession {
    pub queue: Vec<Track>,
    pub index: usize,
    pub position_ms: u64,
}

/// Время, реально проведённое в состоянии воспроизведения.
/// `imported_ms` хранится отдельно, чтобы будущий импорт внешней истории не
/// смешивал оценку с измерением локального плеера.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ListeningStats {
    pub total_ms: u64,
    pub imported_ms: u64,
    pub by_track_ms: HashMap<String, u64>,
}

impl ListeningStats {
    pub fn record(&mut self, track: &Track, delta_ms: u64) {
        if delta_ms == 0 {
            return;
        }
        self.total_ms = self.total_ms.saturating_add(delta_ms);
        let entry = self.by_track_ms.entry(track.id.clone()).or_default();
        *entry = entry.saturating_add(delta_ms);
    }

    pub fn total_with_imported(&self) -> u64 {
        self.total_ms.saturating_add(self.imported_ms)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LibraryData {
    pub favorites: Vec<Track>,
    pub history: Vec<Track>,
    pub playlists: Vec<LocalPlaylist>,
    pub recent_searches: Vec<String>,
    pub listening: ListeningStats,
}

const HISTORY_MAX: usize = 300;

/// Сессия (очередь и позиция) хранится отдельно: она пишется часто.
pub fn save_session(s: Option<&SavedSession>) {
    let path = paths().config.join("session.json");
    match s {
        Some(s) => {
            let _ = write_json(&path, s);
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn load_session() -> Option<SavedSession> {
    read_json(&paths().config.join("session.json"))
}

impl LibraryData {
    pub fn load() -> Self {
        read_json(&paths().config.join("library.json")).unwrap_or_default()
    }

    pub fn save(&self) {
        if let Err(e) = write_json(&paths().config.join("library.json"), self) {
            tracing::warn!("не удалось сохранить библиотеку: {e}");
        }
    }

    pub fn is_favorite(&self, id: &str) -> bool {
        self.favorites.iter().any(|t| t.id == id)
    }

    /// Возвращает новое состояние: true — добавлен.
    pub fn toggle_favorite(&mut self, t: &Track) -> bool {
        if let Some(pos) = self.favorites.iter().position(|x| x.id == t.id) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.insert(0, t.clone());
            true
        }
    }

    pub fn push_history(&mut self, t: &Track) {
        self.history.retain(|x| x.id != t.id);
        self.history.insert(0, t.clone());
        self.history.truncate(HISTORY_MAX);
    }

    pub fn record_listening(&mut self, t: &Track, delta_ms: u64) {
        self.listening.record(t, delta_ms);
    }

    pub fn push_search(&mut self, q: &str) {
        let q = q.trim();
        if q.is_empty() {
            return;
        }
        self.recent_searches.retain(|x| !x.eq_ignore_ascii_case(q));
        self.recent_searches.insert(0, q.to_string());
        self.recent_searches.truncate(12);
    }

    pub fn create_playlist(&mut self, name: &str) -> String {
        let id = format!("local-{}-{}", now_secs(), self.playlists.len());
        self.playlists.push(LocalPlaylist { id: id.clone(), name: name.trim().to_string(), tracks: Vec::new(), created: now_secs() });
        id
    }

    pub fn add_to_playlist(&mut self, pid: &str, t: &Track) -> bool {
        match self.playlists.iter_mut().find(|p| p.id == pid) {
            Some(p) if !p.tracks.iter().any(|x| x.id == t.id) => {
                p.tracks.push(t.clone());
                true
            }
            _ => false,
        }
    }
}

// ───────────────────────────── локальные файлы ─────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct IndexEntry {
    mtime: u64,
    track: Track,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LocalIndex {
    entries: HashMap<String, IndexEntry>,
    pub scanned_at: u64,
}

impl LocalIndex {
    pub fn load() -> Self {
        read_json(&paths().cache.join("local_index.json")).unwrap_or_default()
    }

    fn save(&self) {
        let _ = write_json(&paths().cache.join("local_index.json"), self);
    }

    pub fn tracks(&self) -> Vec<Track> {
        let mut v: Vec<Track> = self.entries.values().map(|e| e.track.clone()).collect();
        v.sort_by(|a, b| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
        v
    }

    /// Инкрементальное сканирование: неизменившиеся файлы берутся из индекса.
    pub fn scan(folders: &[String]) -> LocalIndex {
        let old = LocalIndex::load();
        let mut new = LocalIndex { entries: HashMap::new(), scanned_at: now_secs() };
        let mut stack: Vec<(PathBuf, usize)> = folders.iter().map(|f| (PathBuf::from(f), 0)).collect();
        while let Some((dir, depth)) = stack.pop() {
            if depth > 14 {
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_dir() {
                    stack.push((path, depth + 1));
                    continue;
                }
                let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
                if !AUDIO_EXT.contains(&ext.as_str()) {
                    continue;
                }
                let key = path.display().to_string();
                let mtime = meta.modified().ok().and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
                if let Some(e) = old.entries.get(&key) {
                    if e.mtime == mtime {
                        new.entries.insert(key, e.clone());
                        continue;
                    }
                }
                let track = read_track(&path);
                new.entries.insert(key, IndexEntry { mtime, track });
            }
        }
        new.save();
        new
    }
}

pub fn read_track(path: &Path) -> Track {
    let key = path.display().to_string();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| key.clone());
    let mut t = Track {
        id: format!("file:{key}"),
        title: stem.clone(),
        artist: "Неизвестный исполнитель".into(),
        path: Some(key),
        access: Some("playable".into()),
        ..Default::default()
    };
    // «Исполнитель - Название.mp3» — частый случай без тегов
    if let Some((a, b)) = stem.split_once(" - ") {
        t.artist = a.trim().to_string();
        t.title = b.trim().to_string();
    }
    match lofty::read_from_path(path) {
        Ok(tagged) => {
            t.duration_ms = tagged.properties().duration().as_millis() as u64;
            if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
                if let Some(v) = tag.title().filter(|v| !v.trim().is_empty()) {
                    t.title = v.trim().to_string();
                }
                if let Some(v) = tag.artist().filter(|v| !v.trim().is_empty()) {
                    t.artist = v.trim().to_string();
                }
                t.album = tag.album().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                t.genre = tag.genre().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                t.local_cover = !tag.pictures().is_empty();
            }
        }
        Err(e) => tracing::debug!("теги {}: {e}", path.display()),
    }
    t.uploader = t.artist.clone();
    t
}

/// Встроенная обложка локального файла (кэшируется на диск).
pub fn local_cover(path: &str) -> Option<(Vec<u8>, &'static str)> {
    let cache = paths().covers.join(hash_key(path));
    if let Ok(data) = std::fs::read(&cache) {
        let mime = crate::media::sniff_mime(&data);
        return Some((data, mime));
    }
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    let pic = tag.pictures().first()?;
    let data = pic.data().to_vec();
    let _ = std::fs::write(&cache, &data);
    let mime = crate::media::sniff_mime(&data);
    Some((data, mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_and_history() {
        let mut lib = LibraryData::default();
        let t = Track { id: "sc:1".into(), ..Default::default() };
        assert!(lib.toggle_favorite(&t));
        assert!(lib.is_favorite("sc:1"));
        assert!(!lib.toggle_favorite(&t));
        for i in 0..400 {
            lib.push_history(&Track { id: format!("t{i}"), ..Default::default() });
        }
        lib.push_history(&Track { id: "t399".into(), ..Default::default() });
        assert_eq!(lib.history.len(), HISTORY_MAX);
        assert_eq!(lib.history[0].id, "t399");
        assert_eq!(lib.history.iter().filter(|t| t.id == "t399").count(), 1);
    }

    #[test]
    fn filename_fallback() {
        let t = read_track(Path::new("C:/nonexistent/Artist Name - Song Title.mp3"));
        assert_eq!(t.artist, "Artist Name");
        assert_eq!(t.title, "Song Title");
    }

    #[test]
    fn listening_stats_accumulate_per_track() {
        let mut lib = LibraryData::default();
        let t = Track { id: "sc:1".into(), ..Default::default() };
        lib.record_listening(&t, 1_500);
        lib.record_listening(&t, 500);
        assert_eq!(lib.listening.total_ms, 2_000);
        assert_eq!(lib.listening.by_track_ms.get("sc:1"), Some(&2_000));
    }
}
