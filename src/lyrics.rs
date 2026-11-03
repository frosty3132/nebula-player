//! Синхронизированные тексты песен из LRCLIB (https://lrclib.net).
//!
//! Названия на SoundCloud грязные («Artist - Song (Official Video) [prod. X]»),
//! поэтому сначала чистим их, затем пробуем точный `/api/get`, потом поиск
//! с выбором кандидата по близости длительности. Результаты (в том числе
//! «не найдено») кэшируются на диске, ручной выбор пользователя — тоже.

use crate::model::Track;
use crate::paths::{hash_key, now_secs, paths, read_json, write_json};
use parking_lot::Mutex;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

const API: &str = "https://lrclib.net/api";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LyricLine {
    pub time_ms: u64,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Lyrics {
    pub id: Option<i64>,
    pub track_name: String,
    pub artist_name: String,
    pub duration_s: f64,
    pub synced: Vec<LyricLine>,
    pub plain: Option<String>,
    pub instrumental: bool,
}

impl Lyrics {
    pub fn is_synced(&self) -> bool {
        !self.synced.is_empty()
    }

    /// Индекс активной строки для позиции (мс, уже со сдвигом).
    pub fn active_index(&self, pos_ms: i64) -> Option<usize> {
        if self.synced.is_empty() || pos_ms < 0 {
            return None;
        }
        let pos = pos_ms as u64;
        match self.synced.binary_search_by(|l| l.time_ms.cmp(&pos)) {
            Ok(i) => Some(i),
            Err(0) => None,
            Err(i) => Some(i - 1),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiLyrics {
    id: Option<i64>,
    track_name: Option<String>,
    artist_name: Option<String>,
    duration: Option<f64>,
    instrumental: Option<bool>,
    plain_lyrics: Option<String>,
    synced_lyrics: Option<String>,
}

impl From<ApiLyrics> for Lyrics {
    fn from(a: ApiLyrics) -> Self {
        Lyrics {
            id: a.id,
            track_name: a.track_name.unwrap_or_default(),
            artist_name: a.artist_name.unwrap_or_default(),
            duration_s: a.duration.unwrap_or(0.0),
            synced: a.synced_lyrics.as_deref().map(parse_lrc).unwrap_or_default(),
            plain: a.plain_lyrics.filter(|p| !p.trim().is_empty()),
            instrumental: a.instrumental.unwrap_or(false),
        }
    }
}

/// Разбор LRC: несколько меток на строку, `[offset:±ms]`, метаданные пропускаются.
pub fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut out = Vec::new();
    let mut offset: i64 = 0;
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut times = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            let tag = &stripped[..end];
            rest = stripped[end + 1..].trim_start();
            if let Some(v) = tag.strip_prefix("offset:") {
                offset = v.trim().parse().unwrap_or(0);
                continue;
            }
            if let Some(ms) = parse_stamp(tag) {
                times.push(ms);
            }
        }
        for t in times {
            let time = (t as i64 - offset).max(0) as u64;
            out.push(LyricLine { time_ms: time, text: rest.to_string() });
        }
    }
    out.sort_by_key(|l| l.time_ms);
    out
}

fn parse_stamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let s: f64 = s.trim().replace(',', ".").parse().ok()?;
    Some(m * 60_000 + (s * 1000.0).round() as u64)
}

fn strip_brackets(s: &str, junk: &[&str]) -> String {
    // убираем (…) и […] с «мусорными» словами, музыкальные пометки оставляем
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut buf = String::new();
    for ch in s.chars() {
        match ch {
            '(' | '[' | '【' => {
                if depth == 0 {
                    buf.clear();
                }
                depth += 1;
                buf.push(ch);
            }
            ')' | ']' | '】' if depth > 0 => {
                depth -= 1;
                buf.push(ch);
                if depth == 0 {
                    let lower = buf.to_lowercase();
                    if !junk.iter().any(|j| lower.contains(j)) {
                        out.push_str(&buf);
                    }
                    buf.clear();
                }
            }
            _ if depth > 0 => buf.push(ch),
            _ => out.push(ch),
        }
    }
    out.push_str(&buf);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

const JUNK: &[&str] = &[
    "official", "video", "audio", "lyric", "visualizer", "visualiser", "prod", "hq", "hd", "4k", "free download", "premiere",
    "out now", "clip", "mv", "explicit", "клип", "премьера", "текст", "music video", "full", "feat", "ft.", "remaster",
];

fn first_artist(a: &str) -> String {
    let lower = a.to_lowercase();
    let mut cut = a.len();
    for sep in [",", " & ", " x ", " × ", " feat", " ft.", " ft ", " и ", " vs"] {
        if let Some(p) = lower.find(sep) {
            cut = cut.min(p);
        }
    }
    a[..cut].trim().to_string()
}

/// Угадываем (исполнитель, название) по метаданным трека.
pub fn guess_artist_title(t: &Track) -> (String, String) {
    let mut artist = t.artist.trim().to_string();
    let mut title = t.title.trim().to_string();
    let artist_is_uploader = t.artist == t.uploader || t.artist.is_empty();
    for sep in [" - ", " – ", " — ", " -- ", " ~ "] {
        if let Some((a, b)) = title.split_once(sep) {
            if artist_is_uploader || a.to_lowercase().contains(&first_artist(&artist).to_lowercase()) {
                artist = a.trim().to_string();
                title = b.trim().to_string();
            }
            break;
        }
    }
    let title = strip_brackets(&title, JUNK);
    let title = title.trim_matches(|c: char| c == '"' || c == '«' || c == '»' || c == '-' || c.is_whitespace()).to_string();
    (first_artist(&artist), title)
}

fn norm(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

fn score(c: &Lyrics, artist: &str, title: &str, duration_s: f64) -> Option<f64> {
    if c.synced.is_empty() && c.plain.is_none() && !c.instrumental {
        return None;
    }
    let mut score = 0.0;
    if duration_s > 1.0 && c.duration_s > 1.0 {
        let d = (c.duration_s - duration_s).abs();
        if d > 12.0 {
            return None;
        }
        score -= d * 2.0;
    }
    let (nt, na) = (norm(title), norm(artist));
    let (ct, ca) = (norm(&c.track_name), norm(&c.artist_name));
    if !nt.is_empty() && (ct == nt) {
        score += 30.0;
    } else if !nt.is_empty() && (ct.contains(&nt) || nt.contains(&ct)) {
        score += 15.0;
    } else {
        score -= 20.0;
    }
    if !na.is_empty() && (ca.contains(&na) || na.contains(&ca)) {
        score += 15.0;
    }
    if c.is_synced() {
        score += 12.0;
    }
    Some(score)
}

#[derive(Serialize, Deserialize, Default)]
struct CacheEntry {
    lyrics: Option<Lyrics>,
    fetched: u64,
    manual: bool,
    offset_ms: i32,
}

pub struct LyricsClient {
    http: reqwest::Client,
    mem: Mutex<HashMap<String, Option<Lyrics>>>,
}

fn cache_path(track_id: &str) -> std::path::PathBuf {
    paths().lyrics.join(format!("{}.json", hash_key(track_id)))
}

impl LyricsClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, mem: Mutex::new(HashMap::new()) }
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: Url) -> Result<Option<T>, String> {
        let resp = self
            .http
            .get(url)
            .header(reqwest::header::USER_AGENT, concat!("NebulaPlayer v", env!("CARGO_PKG_VERSION"), " (desktop music player)"))
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|e| format!("lrclib: {e}"))?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(format!("lrclib: HTTP {}", resp.status()));
        }
        resp.json().await.map(Some).map_err(|e| format!("lrclib: {e}"))
    }

    pub async fn search(&self, query: &str) -> Result<Vec<Lyrics>, String> {
        let url = Url::parse_with_params(&format!("{API}/search"), &[("q", query)]).map_err(|e| e.to_string())?;
        let list: Vec<ApiLyrics> = self.get_json(url).await?.unwrap_or_default();
        Ok(list.into_iter().map(Lyrics::from).collect())
    }

    async fn lookup(&self, t: &Track) -> Result<Option<Lyrics>, String> {
        let (artist, title) = guess_artist_title(t);
        if title.is_empty() {
            return Ok(None);
        }
        let dur = t.duration_ms as f64 / 1000.0;

        // 1) точное совпадение
        if dur > 1.0 && !artist.is_empty() {
            let mut params = vec![("artist_name", artist.clone()), ("track_name", title.clone()), ("duration", format!("{}", dur.round() as u64))];
            if let Some(album) = &t.album {
                params.push(("album_name", album.clone()));
            }
            let url = Url::parse_with_params(&format!("{API}/get"), &params).map_err(|e| e.to_string())?;
            if let Some(a) = self.get_json::<ApiLyrics>(url).await? {
                let l = Lyrics::from(a);
                if l.is_synced() || l.plain.is_some() || l.instrumental {
                    return Ok(Some(l));
                }
            }
        }

        // 2) поиск по полям, 3) свободный поиск, 4) только название
        let mut queries: Vec<Vec<(&str, String)>> = Vec::new();
        if !artist.is_empty() {
            queries.push(vec![("track_name", title.clone()), ("artist_name", artist.clone())]);
            queries.push(vec![("q", format!("{artist} {title}"))]);
        }
        if title.chars().count() >= 4 {
            queries.push(vec![("q", title.clone())]);
        }
        for q in queries {
            let url = Url::parse_with_params(&format!("{API}/search"), &q).map_err(|e| e.to_string())?;
            let list: Vec<ApiLyrics> = self.get_json(url).await?.unwrap_or_default();
            let best = list
                .into_iter()
                .map(Lyrics::from)
                .filter_map(|l| score(&l, &artist, &title, dur).map(|s| (s, l)))
                .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((sc, l)) = best {
                if sc > 0.0 {
                    return Ok(Some(l));
                }
            }
        }
        Ok(None)
    }

    /// Текст для трека: память → диск → LRCLIB. «Не найдено» перепроверяется раз в 3 дня.
    pub async fn for_track(&self, t: &Track) -> Result<Option<Lyrics>, String> {
        if let Some(hit) = self.mem.lock().get(&t.id).cloned() {
            return Ok(hit);
        }
        let path = cache_path(&t.id);
        if let Some(entry) = read_json::<CacheEntry>(&path) {
            if entry.lyrics.is_some() || entry.manual || now_secs().saturating_sub(entry.fetched) < 3 * 86_400 {
                self.mem.lock().insert(t.id.clone(), entry.lyrics.clone());
                return Ok(entry.lyrics);
            }
        }
        let found = self.lookup(t).await?;
        let offset = read_json::<CacheEntry>(&path).map(|e| e.offset_ms).unwrap_or(0);
        let _ = write_json(&path, &CacheEntry { lyrics: found.clone(), fetched: now_secs(), manual: false, offset_ms: offset });
        self.mem.lock().insert(t.id.clone(), found.clone());
        Ok(found)
    }

    /// Ручной выбор текста из результатов поиска.
    pub fn set_manual(&self, track_id: &str, lyrics: Option<Lyrics>) {
        let path = cache_path(track_id);
        let offset = read_json::<CacheEntry>(&path).map(|e| e.offset_ms).unwrap_or(0);
        let _ = write_json(&path, &CacheEntry { lyrics: lyrics.clone(), fetched: now_secs(), manual: true, offset_ms: offset });
        self.mem.lock().insert(track_id.to_string(), lyrics);
    }

    pub fn offset(&self, track_id: &str) -> i32 {
        read_json::<CacheEntry>(&cache_path(track_id)).map(|e| e.offset_ms).unwrap_or(0)
    }

    pub fn set_offset(&self, track_id: &str, offset_ms: i32) {
        let path = cache_path(track_id);
        let mut e = read_json::<CacheEntry>(&path).unwrap_or_default();
        e.offset_ms = offset_ms;
        if e.fetched == 0 {
            e.fetched = now_secs();
            e.lyrics = self.mem.lock().get(track_id).cloned().flatten();
        }
        let _ = write_json(&path, &e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_parsing() {
        let l = parse_lrc("[ar:Someone]\n[00:12.50]Hello\n[00:05.00][00:20.10]Twice\n[01:02.003]End\n[00:30.00]\n");
        let times: Vec<_> = l.iter().map(|x| x.time_ms).collect();
        assert_eq!(times, vec![5000, 12500, 20100, 30000, 62003]);
        assert_eq!(l[0].text, "Twice");
        assert_eq!(l[3].text, "");
    }

    #[test]
    fn lrc_offset() {
        let l = parse_lrc("[offset:+500]\n[00:01.00]A\n");
        assert_eq!(l[0].time_ms, 500);
    }

    #[test]
    fn active_line() {
        let l = Lyrics { synced: parse_lrc("[00:01.00]a\n[00:03.00]b\n[00:05.00]c"), ..Default::default() };
        assert_eq!(l.active_index(500), None);
        assert_eq!(l.active_index(1000), Some(0));
        assert_eq!(l.active_index(4000), Some(1));
        assert_eq!(l.active_index(99_000), Some(2));
    }

    #[test]
    fn title_cleanup() {
        let t = Track { title: "Linkin Park - Numb (Official Video) [HD]".into(), artist: "linkinparkofficial".into(), uploader: "linkinparkofficial".into(), ..Default::default() };
        assert_eq!(guess_artist_title(&t), ("Linkin Park".into(), "Numb".into()));
        let t = Track { title: "Song (Remix)".into(), artist: "DJ A & B".into(), uploader: "label".into(), ..Default::default() };
        assert_eq!(guess_artist_title(&t), ("DJ A".into(), "Song (Remix)".into()));
        let t = Track { title: "Мой трек (feat. Кто-то)".into(), artist: "Артист".into(), uploader: "Артист".into(), ..Default::default() };
        assert_eq!(guess_artist_title(&t), ("Артист".into(), "Мой трек".into()));
    }

    #[test]
    fn scoring_prefers_close_duration_and_synced() {
        let a = Lyrics { track_name: "Numb".into(), artist_name: "Linkin Park".into(), duration_s: 186.0, synced: parse_lrc("[00:01.00]x"), ..Default::default() };
        let b = Lyrics { track_name: "Numb".into(), artist_name: "Linkin Park".into(), duration_s: 230.0, plain: Some("x".into()), ..Default::default() };
        assert!(score(&a, "Linkin Park", "Numb", 187.0).unwrap() > 0.0);
        assert!(score(&b, "Linkin Park", "Numb", 187.0).is_none());
    }
}
