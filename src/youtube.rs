//! YouTube через yt-dlp: поиск, прямые ссылки на аудиопоток и подбор полной версии
//! для треков SoundCloud, от которых доступно только 30-секундное превью.
//!
//! yt-dlp — отдельная программа (Python), поэтому мы запускаем `yt-dlp.exe` как
//! дочерний процесс. Где его ищем:
//! 1. рядом с `Nebula.exe` (можно положить свою копию);
//! 2. в кэше приложения `bin\yt-dlp.exe` — туда он скачивается сам при первом
//!    обращении и раз в неделю обновляется (`yt-dlp -U`): YouTube часто ломает
//!    старые версии, и так плееру не нужен новый релиз ради свежего yt-dlp.
//!
//! Играем только AAC/M4A (`bestaudio[ext=m4a]`): его умеет наш движок на symphonia,
//! ffmpeg не нужен. Ссылки googlevideo живут несколько часов — держим их в памяти
//! до истечения `expire`, а сопоставления «трек SoundCloud → видео» — на диске.

use crate::audio::MediaSpec;
use crate::model::{Page, Track};
use crate::paths::{now_secs, paths, read_json, write_json};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

#[cfg(windows)]
const EXE: &str = "yt-dlp.exe";
#[cfg(not(windows))]
const EXE: &str = "yt-dlp";

#[cfg(windows)]
const RELEASE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
#[cfg(not(windows))]
const RELEASE_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";

/// Аудио без перекодирования: AAC в MP4 (itag 140 и родственные).
const AUDIO_FORMAT: &str = "bestaudio[ext=m4a]/bestaudio[acodec^=mp4a]";
const PAGE: usize = 25;
const MAX_RESULTS: usize = 150;
const UPDATE_EVERY: u64 = 7 * 24 * 3600;
/// «Не нашли» перепроверяем через 3 дня — видео могли загрузить.
const MISS_TTL: u64 = 3 * 24 * 3600;

/// Windows: «распакованная» сборка yt-dlp (папка с exe). Однофайловый yt-dlp.exe
/// при каждом запуске распаковывает себя во временную папку — это ~1,5 с на
/// каждый вызов; папочная сборка стартует сразу. Сама она не обновляется —
/// раз в неделю сверяем версию с последним релизом на GitHub.
#[cfg(windows)]
const ONEDIR_URL: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_win.zip";
const LATEST_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct YtStore {
    last_update: u64,
    /// Папка установленной быстрой сборки в `cache\bin` и её версия.
    onedir: Option<String>,
    onedir_version: String,
    /// sc-id трека → (id видео или None, когда проверяли)
    matches: HashMap<String, (Option<String>, u64)>,
    /// Версия правил сопоставления: после их улучшения старые «не нашли» перепроверяются.
    matcher: u32,
    /// Ссылки на потоки (id видео → ссылка, годна до) — переживают перезапуск.
    urls: HashMap<String, (String, u64)>,
}

/// 2 — полная длительность превью SoundCloud, бонус за канал любого из исполнителей.
const MATCHER_VERSION: u32 = 2;

fn load_store() -> YtStore {
    let mut s: YtStore = read_json(&store_path()).unwrap_or_default();
    // просроченные ссылки на потоки не нужны
    let now = now_secs();
    s.urls.retain(|_, (_, until)| *until > now);
    if s.matcher < MATCHER_VERSION {
        s.matches.retain(|_, (hit, _)| hit.is_some());
        s.matcher = MATCHER_VERSION;
        let _ = write_json(&store_path(), &s);
    }
    s
}

pub struct YouTube {
    http: reqwest::Client,
    store: Mutex<YtStore>,
    /// id видео → (прямая ссылка, до какого времени годна)
    urls: Mutex<HashMap<String, (String, u64)>>,
    install: tokio::sync::Mutex<()>,
    /// Фоновый подбор полных версий: что уже в работе и сколько одновременно.
    prematch_busy: Mutex<HashSet<String>>,
    prematch_slots: tokio::sync::Semaphore,
}

fn bin_dir() -> PathBuf {
    paths().cache.join("bin")
}

/// exe внутри распакованной сборки (в корне архива или в его единственной папке).
fn find_exe(dir: &Path) -> Option<PathBuf> {
    let direct = dir.join(EXE);
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path().join(EXE)).find(|p| p.is_file())
}

/// Ключ сопоставления трека SoundCloud с видео.
fn match_key(t: &Track) -> String {
    t.urn.clone().unwrap_or_else(|| t.id.clone())
}

fn store_path() -> PathBuf {
    paths().config.join("youtube.json")
}

fn managed_bin() -> PathBuf {
    paths().cache.join("bin").join(EXE)
}

fn bundled_bin() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join(EXE);
    p.is_file().then_some(p)
}

fn str_of(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn video_id(t: &Track) -> Option<&str> {
    t.id.strip_prefix("yt:")
}

pub fn watch_url(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

/// Срок жизни ссылки googlevideo из параметра `expire` (минус запас), иначе 4 часа.
fn url_deadline(url: &str) -> u64 {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == "expire").and_then(|(_, v)| v.parse::<u64>().ok()))
        .map(|e| e.saturating_sub(20 * 60))
        .unwrap_or_else(|| now_secs() + 4 * 3600)
}

/// Запись выдачи yt-dlp (`--flat-playlist` или полный `-j`) → наш трек.
pub fn track_from(v: &Value) -> Option<Track> {
    let id = str_of(v, "id")?;
    // прямые эфиры и премьеры без длительности не играем
    let duration = v.get("duration").and_then(Value::as_f64).filter(|d| *d > 0.0)?;
    if v.get("live_status").and_then(Value::as_str).is_some_and(|s| s == "is_live" || s == "is_upcoming") {
        return None;
    }
    let raw_title = str_of(v, "title").unwrap_or_else(|| "Без названия".into());
    let channel = str_of(v, "channel").or_else(|| str_of(v, "uploader")).unwrap_or_default();
    let channel_clean = channel.strip_suffix(" - Topic").unwrap_or(&channel).to_string();
    // «Исполнитель - Название» — так тексты и обложки ищутся гораздо лучше
    let (artist, title) = match str_of(v, "artist").or_else(|| str_of(v, "creator")) {
        Some(a) => (a, str_of(v, "track").unwrap_or(raw_title)),
        None => {
            let split = raw_title
                .split_once(" - ")
                .or_else(|| raw_title.split_once(" – "))
                .map(|(a, t)| (a.trim().to_string(), t.trim().to_string()))
                .filter(|(a, t)| !a.is_empty() && !t.is_empty());
            split.unwrap_or_else(|| (channel_clean.clone(), raw_title.clone()))
        }
    };
    Some(Track {
        id: format!("yt:{id}"),
        title,
        artist,
        uploader: channel_clean,
        duration_ms: (duration * 1000.0) as u64,
        artwork: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
        permalink: Some(watch_url(&id)),
        plays: v.get("view_count").and_then(Value::as_u64),
        likes: v.get("like_count").and_then(Value::as_u64),
        access: Some("playable".into()),
        ..Default::default()
    })
}

// ───────────────────────────── сопоставление ─────────────────────────────

const JUNK: &[&str] = &[
    "live", "cover", "remix", "sped", "slowed", "nightcore", "karaoke", "8d", "reverb", "instrumental", "reaction", "loop", "bass boosted", "sped up", "1 hour",
    "teaser", "тизер", "snippet", "сниппет", "preview",
];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 2 && !matches!(*w, "ft" | "feat" | "official" | "video" | "audio" | "music" | "the" | "prod"))
        .map(str::to_string)
        .collect()
}

/// Поисковый запрос по треку SoundCloud: в названиях часто уже есть исполнитель
/// и мусор вроде «[FREE DL]» — убираем скобки и не дублируем артиста.
pub fn match_query(t: &Track) -> String {
    let mut title = String::with_capacity(t.title.len());
    let mut depth = 0i32;
    for c in t.title.chars() {
        match c {
            '[' | '【' => depth += 1,
            ']' | '】' => depth -= 1,
            _ if depth <= 0 => title.push(c),
            _ => {}
        }
    }
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    let artist = t.artist.trim();
    if artist.is_empty() || title.to_lowercase().contains(&artist.to_lowercase()) {
        title
    } else {
        format!("{artist} {title}")
    }
}

/// Оценка кандидата: длительность (главное), совпадение названия и исполнителя,
/// штраф за ремиксы/live/nightcore. `None` — точно не тот трек.
pub fn score(sc: &Track, cand: &Track) -> Option<i32> {
    let mut score = 0i32;
    if sc.duration_ms > 60_000 {
        let diff = sc.duration_ms.abs_diff(cand.duration_ms);
        score += match diff {
            0..=3_000 => 45,
            3_001..=8_000 => 30,
            8_001..=20_000 => 12,
            20_001..=45_000 => 0,
            _ => return None,
        };
    }
    let have: HashSet<String> = words(&format!("{} {} {}", cand.artist, cand.title, cand.uploader)).into_iter().collect();
    let ratio = |ws: &[String]| ws.iter().filter(|w| have.contains(*w)).count() as f32 / ws.len().max(1) as f32;
    let artist_words = words(&sc.artist);
    let title_words: Vec<String> = words(&match_query(sc)).into_iter().filter(|w| !artist_words.contains(w)).collect();
    // название обязано совпасть: один исполнитель без названия — это другой его трек
    if !title_words.is_empty() {
        let r = ratio(title_words.as_slice());
        if r < 0.6 {
            return None;
        }
        score += (r * 35.0) as i32;
    }
    if !artist_words.is_empty() {
        score += (ratio(artist_words.as_slice()) * 15.0) as i32;
    }
    if title_words.is_empty() && artist_words.is_empty() {
        return None;
    }
    let sc_words: HashSet<String> = words(&sc.title).into_iter().collect();
    let sc_low = sc.title.to_lowercase();
    let cand_low = format!("{} {}", cand.title, cand.artist).to_lowercase();
    for j in JUNK {
        let in_cand = if j.contains(' ') { cand_low.contains(j) } else { have.contains(*j) };
        let in_sc = if j.contains(' ') { sc_low.contains(j) } else { sc_words.contains(*j) };
        if in_cand && !in_sc {
            score -= 35;
        }
    }
    // канал исполнителя (любого из перечисленных) или авто-канал «Artist - Topic»
    let uploader = cand.uploader.to_lowercase();
    if !uploader.is_empty() && artist_names(&sc.artist).iter().any(|a| *a == uploader) {
        score += 10;
    }
    Some(score)
}

/// Исполнители из строки вида «A, B & C feat. D» (в нижнем регистре).
fn artist_names(artist: &str) -> Vec<String> {
    let low = artist.to_lowercase();
    let mut names = vec![low.trim().to_string()];
    let parts = low.replace(" feat. ", ",").replace(" ft. ", ",").replace(" x ", ",").replace(['&', ';', '/'], ",");
    names.extend(parts.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string));
    names
}

pub fn best_match<'a>(sc: &Track, cands: &'a [Track]) -> Option<&'a Track> {
    // без длительности (её у трека нет или она подозрительно короткая) главный
    // признак недоступен — достаточно совпадения названия и исполнителя
    let min = if sc.duration_ms > 60_000 { 55 } else { 45 };
    cands
        .iter()
        .filter_map(|c| score(sc, c).map(|s| (s, c)))
        .filter(|(s, _)| *s >= min)
        .max_by_key(|(s, _)| *s)
        .map(|(_, c)| c)
}

// ───────────────────────────── клиент ─────────────────────────────

impl YouTube {
    pub fn new(http: reqwest::Client) -> Self {
        let store = load_store();
        let urls = store.urls.clone();
        Self {
            http,
            store: Mutex::new(store),
            urls: Mutex::new(urls),
            install: tokio::sync::Mutex::new(()),
            prematch_busy: Mutex::new(HashSet::new()),
            prematch_slots: tokio::sync::Semaphore::new(2),
        }
    }

    fn save(&self) {
        let s = self.store.lock().clone();
        if let Err(e) = write_json(&store_path(), &s) {
            tracing::warn!("youtube.json: {e}");
        }
    }

    /// Exe установленной быстрой сборки, если она есть.
    fn onedir_exe(&self) -> Option<PathBuf> {
        let name = self.store.lock().onedir.clone()?;
        find_exe(&bin_dir().join(name))
    }

    /// Поставить (или обновить) быструю сборку yt-dlp. Если уже стоит последняя
    /// версия — ничего не качает. Возвращает путь к exe.
    #[cfg(windows)]
    async fn install_onedir(&self) -> Result<PathBuf, String> {
        let _guard = self.install.lock().await;
        let latest = self
            .http
            .get(LATEST_API)
            .header("Accept", "application/vnd.github+json")
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .ok()
            .filter(|r| r.status().is_success());
        let latest = match latest {
            Some(r) => r.json::<Value>().await.ok().and_then(|v| str_of(&v, "tag_name")),
            None => None,
        };
        let installed = self.store.lock().onedir_version.clone();
        if let (Some(exe), Some(tag)) = (self.onedir_exe(), latest.as_deref()) {
            if tag == installed {
                self.store.lock().last_update = now_secs();
                self.save();
                return Ok(exe);
            }
        }
        let tag = latest.unwrap_or_else(|| format!("{}", now_secs()));
        tracing::info!("скачиваю быструю сборку yt-dlp {tag}");
        let resp = self
            .http
            .get(ONEDIR_URL)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| format!("не удалось скачать yt-dlp: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("не удалось скачать yt-dlp: HTTP {}", resp.status()));
        }
        let data = resp.bytes().await.map_err(|e| format!("не удалось скачать yt-dlp: {e}"))?;
        if data.len() < 1_000_000 {
            return Err("скачанный yt-dlp повреждён".into());
        }
        // в новую папку: старая может быть занята работающим yt-dlp
        let name = format!("yt-dlp_win-{}", tag.replace(|c: char| !c.is_ascii_alphanumeric() && c != '.', "_"));
        let dir = bin_dir().join(&name);
        let target = dir.clone();
        tokio::task::spawn_blocking(move || -> Result<(), String> {
            let _ = std::fs::remove_dir_all(&target);
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("архив yt-dlp повреждён: {e}"))?;
            zip.extract(&target).map_err(|e| format!("не удалось распаковать yt-dlp: {e}"))
        })
        .await
        .map_err(|e| e.to_string())??;
        let exe = find_exe(&dir).ok_or("в архиве yt-dlp нет yt-dlp.exe")?;
        Self::exec(&exe, &["--version"], Duration::from_secs(30)).await?;
        let old = {
            let mut s = self.store.lock();
            let old = s.onedir.replace(name.clone());
            s.onedir_version = tag;
            s.last_update = now_secs();
            old
        };
        self.save();
        // прежние версии убираем, если они уже не заняты
        if let Some(old) = old.filter(|o| *o != name) {
            let _ = std::fs::remove_dir_all(bin_dir().join(old));
        }
        Ok(exe)
    }

    /// Путь к yt-dlp. Windows: быстрая сборка (ставится и раз в неделю
    /// обновляется в фоне; пока её нет — работает однофайловый exe).
    async fn binary(&self) -> Result<PathBuf, String> {
        if let Some(p) = bundled_bin() {
            return Ok(p);
        }
        #[cfg(windows)]
        {
            let stale = now_secs().saturating_sub(self.store.lock().last_update) > UPDATE_EVERY;
            if let Some(exe) = self.onedir_exe() {
                if stale && self.install.try_lock().is_ok() {
                    self.store.lock().last_update = now_secs();
                    crate::services::svc().rt.spawn(async {
                        if let Err(e) = crate::services::svc().yt.install_onedir().await {
                            tracing::info!("обновление yt-dlp: {e}");
                        }
                    });
                }
                return Ok(exe);
            }
            if managed_bin().is_file() {
                // старый однофайловый — пока ставится быстрая сборка
                if self.install.try_lock().is_ok() {
                    crate::services::svc().rt.spawn(async {
                        if let Err(e) = crate::services::svc().yt.install_onedir().await {
                            tracing::info!("быстрая сборка yt-dlp: {e}");
                        }
                    });
                }
                return Ok(managed_bin());
            }
            return self.install_onedir().await;
        }
        #[allow(unreachable_code)]
        let path = managed_bin();
        let _guard = self.install.lock().await;
        let last_update = self.store.lock().last_update;
        if !path.is_file() {
            self.download(&path).await?;
            self.store.lock().last_update = now_secs();
            self.save();
        } else if now_secs().saturating_sub(last_update) > UPDATE_EVERY {
            self.store.lock().last_update = now_secs();
            self.save();
            // обновление в фоне: текущий запрос не ждёт
            let p = path.clone();
            tokio::spawn(async move {
                let _ = Self::exec(&p, &["-U"], Duration::from_secs(120)).await;
            });
        }
        Ok(path)
    }

    async fn download(&self, path: &Path) -> Result<(), String> {
        tracing::info!("скачиваю yt-dlp в {}", path.display());
        if let Some(dir) = path.parent() {
            tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
        }
        let resp = self
            .http
            .get(RELEASE_URL)
            .timeout(Duration::from_secs(180))
            .send()
            .await
            .map_err(|e| format!("не удалось скачать yt-dlp: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("не удалось скачать yt-dlp: HTTP {}", resp.status()));
        }
        let data = resp.bytes().await.map_err(|e| format!("не удалось скачать yt-dlp: {e}"))?;
        if data.len() < 1_000_000 {
            return Err("скачанный yt-dlp повреждён".into());
        }
        let tmp = path.with_extension("part");
        tokio::fs::write(&tmp, &data).await.map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
        }
        tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string())
    }

    async fn exec(bin: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
        let mut cmd = Command::new(bin);
        cmd.args(["--ignore-config", "--no-warnings", "--encoding", "utf-8"])
            .args(args)
            .env("PYTHONIOENCODING", "utf-8")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // без мелькающего окна консоли
        #[cfg(windows)]
        {
            cmd.creation_flags(0x0800_0000);
        }
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| "yt-dlp не ответил вовремя".to_string())?
            .map_err(|e| format!("не удалось запустить yt-dlp: {e}"))?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            let line = err.lines().rev().find(|l| l.contains("ERROR")).or_else(|| err.lines().last()).unwrap_or("неизвестная ошибка");
            return Err(format!("yt-dlp: {}", line.trim_start_matches("ERROR:").trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    async fn run(&self, args: &[&str], timeout: Duration) -> Result<String, String> {
        let bin = self.binary().await?;
        Self::exec(&bin, args, timeout).await
    }

    /// Обновить yt-dlp сейчас (кнопка в настройках). Возвращает версию.
    pub async fn update(&self) -> Result<String, String> {
        #[cfg(windows)]
        if bundled_bin().is_none() {
            self.install_onedir().await?;
            return self.version().await;
        }
        let bin = self.binary().await?;
        if bundled_bin().is_none() {
            Self::exec(&bin, &["-U"], Duration::from_secs(180)).await?;
            self.store.lock().last_update = now_secs();
            self.save();
        }
        self.version().await
    }

    pub async fn version(&self) -> Result<String, String> {
        Ok(self.run(&["--version"], Duration::from_secs(30)).await?.trim().to_string())
    }

    async fn search_range(&self, q: &str, start: usize, end: usize) -> Result<Vec<Track>, String> {
        let target = format!("ytsearch{end}:{q}");
        let items = format!("{start}-{end}");
        let out = self.run(&["--flat-playlist", "-J", "--playlist-items", items.as_str(), target.as_str()], Duration::from_secs(45)).await?;
        let v: Value = serde_json::from_str(&out).map_err(|e| format!("yt-dlp вернул странный ответ: {e}"))?;
        Ok(v.get("entries").and_then(Value::as_array).map(|a| a.iter().filter_map(track_from).collect()).unwrap_or_default())
    }

    /// `next` для «Загрузить ещё»: запрос и номер следующего результата.
    fn page(q: &str, start: usize, items: Vec<Track>) -> Page<Track> {
        let end = start + PAGE - 1;
        let next = (end < MAX_RESULTS).then(|| format!("{}\n{q}", end + 1));
        Page { items, next }
    }

    pub async fn search(&self, q: &str) -> Result<Page<Track>, String> {
        let items = self.search_range(q, 1, PAGE).await?;
        Ok(Self::page(q, 1, items))
    }

    pub async fn next_tracks(&self, next: &str) -> Result<Page<Track>, String> {
        let (start, q) = next.split_once('\n').ok_or("плохой курсор")?;
        let start: usize = start.parse().map_err(|_| "плохой курсор")?;
        let items = self.search_range(q, start, start + PAGE - 1).await?;
        Ok(Self::page(q, start, items))
    }

    /// Прямая ссылка на M4A-поток видео.
    pub async fn stream_id(&self, id: &str) -> Result<MediaSpec, String> {
        let cached = self.urls.lock().get(id).cloned();
        if let Some((url, until)) = cached {
            if now_secs() < until {
                return Ok(MediaSpec::Http { url, auth: None });
            }
        }
        let page = watch_url(id);
        let out = self.run(&["-f", AUDIO_FORMAT, "--no-playlist", "-O", "url", page.as_str()], Duration::from_secs(60)).await?;
        let url = out.lines().map(str::trim).find(|l| l.starts_with("http")).ok_or("yt-dlp не нашёл аудиопоток M4A")?.to_string();
        let entry = (url.clone(), url_deadline(&url));
        self.urls.lock().insert(id.to_string(), entry.clone());
        // на диск — чтобы после перезапуска тот же трек начинался сразу
        self.store.lock().urls.insert(id.to_string(), entry);
        self.save();
        Ok(MediaSpec::Http { url, auth: None })
    }

    pub async fn stream(&self, t: &Track) -> Result<MediaSpec, String> {
        self.stream_id(video_id(t).ok_or("это не трек YouTube")?).await
    }

    /// Полная версия трека SoundCloud на YouTube (id видео) с кэшем на диске.
    pub async fn find_match(&self, t: &Track) -> Result<Option<String>, String> {
        let key = match_key(t);
        let cached = self.store.lock().matches.get(&key).cloned();
        if let Some((hit, at)) = cached {
            if hit.is_some() || now_secs().saturating_sub(at) < MISS_TTL {
                return Ok(hit);
            }
        }
        let cands = self.search_range(&match_query(t), 1, 8).await?;
        let hit = best_match(t, &cands).and_then(|c| video_id(c).map(str::to_string));
        tracing::info!("YouTube для «{} — {}»: {:?}", t.artist, t.title, hit);
        self.store.lock().matches.insert(key, (hit.clone(), now_secs()));
        self.save();
        Ok(hit)
    }

    /// Есть ли уже свежий ответ «какое видео» для трека (найдено или недавно не нашли).
    fn match_known(&self, key: &str) -> bool {
        self.store.lock().matches.get(key).is_some_and(|(hit, at)| hit.is_some() || now_secs().saturating_sub(*at) < MISS_TTL)
    }

    /// Заранее подобрать полные версии для превью и недоступных треков из
    /// открытого списка — тогда по клику остаётся только взять ссылку на поток.
    /// Не больше 12 за раз и по 2 одновременно, уже известные пропускаются.
    pub fn prematch(&'static self, tracks: Vec<Track>) {
        for t in tracks.into_iter().filter(|t| (t.is_preview() || t.is_blocked()) && video_id(t).is_none()).take(12) {
            let key = match_key(&t);
            if self.match_known(&key) || !self.prematch_busy.lock().insert(key.clone()) {
                continue;
            }
            crate::services::svc().rt.spawn(async move {
                if let Ok(_slot) = self.prematch_slots.acquire().await {
                    if !self.match_known(&key) {
                        let _ = self.find_match(&t).await;
                    }
                }
                self.prematch_busy.lock().remove(&key);
            });
        }
    }

    /// Поток полной версии вместо превью SoundCloud.
    pub async fn replacement(&self, t: &Track) -> Result<MediaSpec, String> {
        match self.find_match(t).await? {
            Some(id) => self.stream_id(&id).await,
            None => Err("на YouTube не нашлось подходящей полной версии".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(artist: &str, title: &str, secs: u64) -> Track {
        Track { id: "sc:x".into(), artist: artist.into(), title: title.into(), duration_ms: secs * 1000, access: Some("preview".into()), ..Default::default() }
    }

    fn yt(id: &str, title: &str, channel: &str, secs: f64) -> Track {
        track_from(&serde_json::json!({"id": id, "title": title, "channel": channel, "duration": secs})).unwrap()
    }

    #[test]
    fn parses_flat_entries() {
        let t = yt("abc", "Daft Punk - Get Lucky (Official Audio)", "Daft Punk", 248.0);
        assert_eq!((t.id.as_str(), t.artist.as_str(), t.title.as_str(), t.duration_ms), ("yt:abc", "Daft Punk", "Get Lucky (Official Audio)", 248_000));
        let t = yt("x", "Get Lucky", "Daft Punk - Topic", 248.0);
        assert_eq!((t.artist.as_str(), t.uploader.as_str()), ("Daft Punk", "Daft Punk"));
        assert!(track_from(&serde_json::json!({"id": "live", "title": "stream", "duration": null})).is_none());
    }

    #[test]
    fn query_strips_tags() {
        assert_eq!(match_query(&sc("Daft Punk", "Get Lucky [FREE DL]", 248)), "Daft Punk Get Lucky");
        assert_eq!(match_query(&sc("Daft Punk", "Daft Punk - Get Lucky", 248)), "Daft Punk - Get Lucky");
    }

    #[test]
    fn picks_original_over_junk() {
        let s = sc("Daft Punk", "Get Lucky", 248);
        let cands = vec![
            yt("n", "Daft Punk - Get Lucky (Nightcore)", "nc", 200.0),
            yt("l", "Daft Punk - Get Lucky (Live)", "x", 251.0),
            yt("o", "Get Lucky", "Daft Punk - Topic", 248.0),
            yt("z", "Something Else", "Daft Punk", 248.0),
        ];
        assert!(score(&s, &cands[3]).is_none(), "другой трек того же исполнителя");
        assert_eq!(best_match(&s, &cands).map(|t| t.id.as_str()), Some("yt:o"));
        // слишком разная длительность — не берём
        let cands = vec![yt("m", "Daft Punk - Get Lucky (Extended Mix)", "x", 600.0)];
        assert!(best_match(&s, &cands).is_none());
    }

    /// Настоящая выдача YouTube для превью с несколькими исполнителями.
    #[test]
    fn matches_multi_artist_track() {
        let artist = "НАВЕРНОЕ ПОЭТ, ЯКОРЪ, EVEN CUTE, Ernest Merkel";
        let cands = vec![
            yt("lyp", "ДА, Я РУССКИЙ (PHONK Version) · НАВЕРНОЕ ПОЭТ · ЯКОРЪ · EVEN CUTE · Ernest Merkel(Slowed + Reverb)", "vestkrat music", 146.0),
            yt("tz", "НАВЕРНОЕ ПОЭТ, ЯКОРЪ, EVEN CUTE, ERNEST MERKEL — ДА, Я РУССКИЙ (2024, ТИЗЕР КЛИПА)", "Ernest Merkel", 67.0),
            yt("hk", "НАВЕРНОЕ ПОЭТ, ЯКОРЪ, EVEN CUTE, Ernest Merkel - ДА, Я РУССКИЙ", "Hakii", 115.0),
            yt("ya", "НАВЕРНОЕ ПОЭТ, ЯКОРЪ, EVEN CUTE, Ernest Merkel - ДА, Я РУССКИЙ (ПОЛНАЯ ВЕРСИЯ)", "ЯКОРЪ", 133.0),
        ];
        // длительность неизвестна (старое превью на 30 с): решают название и канал исполнителя
        let pick = best_match(&sc(artist, "ДА, Я РУССКИЙ", 30), &cands).map(|t| t.id.as_str());
        assert_eq!(pick, Some("yt:ya"));
        // с полной длительностью побеждает совпадающая по длине версия, тизер и slowed — нет
        let pick = best_match(&sc(artist, "ДА, Я РУССКИЙ", 115), &cands).map(|t| t.id.as_str());
        assert_eq!(pick, Some("yt:hk"));
    }

    #[test]
    fn deadline_from_expire() {
        assert_eq!(url_deadline("https://r1.googlevideo.com/videoplayback?expire=1800000000&x=1"), 1_800_000_000 - 1200);
    }
}
