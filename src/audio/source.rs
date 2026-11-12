//! Источники байтов для декодера.
//!
//! * [`ProgBuffer`] — прогрессивная HTTP-загрузка в растущий буфер (с докачкой по Range).
//! * [`HlsBuffer`]  — HLS: плейлист + сегменты, которые качаются лениво в окне
//!   «чтения вперёд» вокруг текущей позиции. Старые сегменты длинных миксов
//!   вытесняются из памяти. Перемотка = переход к нужному сегменту.
//!
//! Читатели (`ProgReader`, `HlsReader`) блокируются, пока нужных данных нет,
//! но регулярно проверяют флаг прерывания — перемотка и смена трека не зависают
//! на медленной сети.

use super::hls::{self, Playlist};
use super::ts;
use parking_lot::{Condvar, Mutex};
use reqwest::header::{AUTHORIZATION, RANGE};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::io::MediaSource;
use tokio::runtime::Handle;
use tokio::sync::Notify;

/// Формат байтового потока, который увидит symphonia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Unknown,
    Mp3,
    Adts,
    Mp4,
    Ogg,
}

impl Container {
    pub fn ext(self) -> Option<&'static str> {
        match self {
            Container::Mp3 => Some("mp3"),
            Container::Adts => Some("aac"),
            Container::Mp4 => Some("mp4"),
            Container::Ogg => Some("ogg"),
            Container::Unknown => None,
        }
    }
}

pub fn aborted() -> io::Error {
    io::Error::other("nebula: чтение прервано")
}

pub fn is_aborted(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::Other && e.to_string().contains("nebula: чтение прервано")
}

/// Заголовок авторизации отправляем только на домены SoundCloud.
fn auth_for(url: &str, auth: &Option<String>) -> Option<String> {
    let a = auth.as_ref()?;
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_ascii_lowercase();
    (host.ends_with("soundcloud.com") || host.ends_with("soundcloud.cloud")).then(|| a.clone())
}

fn strip_id3(mut d: &[u8]) -> &[u8] {
    while d.len() >= 10 && &d[..3] == b"ID3" {
        let size = ((d[6] as usize & 0x7f) << 21)
            | ((d[7] as usize & 0x7f) << 14)
            | ((d[8] as usize & 0x7f) << 7)
            | (d[9] as usize & 0x7f);
        let footer = if d[5] & 0x10 != 0 { 10 } else { 0 };
        let total = 10 + size + footer;
        if total > d.len() {
            break;
        }
        d = &d[total..];
    }
    d
}

pub fn detect(d: &[u8]) -> Container {
    let d = strip_id3(d);
    if d.len() >= 8 && matches!(&d[4..8], b"ftyp" | b"styp" | b"moof" | b"moov" | b"sidx" | b"mdat") {
        return Container::Mp4;
    }
    if d.starts_with(b"OggS") {
        return Container::Ogg;
    }
    if d.len() >= 2 && d[0] == 0xff {
        if d[1] & 0xf6 == 0xf0 {
            return Container::Adts;
        }
        if d[1] & 0xe0 == 0xe0 {
            return Container::Mp3;
        }
    }
    Container::Unknown
}

fn is_ts(d: &[u8]) -> bool {
    d.first() == Some(&0x47) && (d.len() < 189 || d[188] == 0x47)
}

pub(crate) async fn fetch_bytes(http: &reqwest::Client, url: &str, auth: &Option<String>, range: Option<hls::ByteRange>) -> Result<Vec<u8>, String> {
    let mut last = String::new();
    for attempt in 0..4u64 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(350 * attempt)).await;
        }
        let mut req = http.get(url);
        if let Some(a) = auth_for(url, auth) {
            req = req.header(AUTHORIZATION, a);
        }
        if let Some((off, len)) = range {
            req = req.header(RANGE, format!("bytes={}-{}", off, off + len.max(1) - 1));
        }
        match req.send().await {
            Ok(resp) => {
                let status = resp.status();
                if status.is_client_error() && status.as_u16() != 429 && status.as_u16() != 408 {
                    return Err(format!("HTTP {status}"));
                }
                if !status.is_success() {
                    last = format!("HTTP {status}");
                    continue;
                }
                let partial = status.as_u16() == 206;
                match resp.bytes().await {
                    Ok(b) => {
                        // сервер проигнорировал Range — вырезаем кусок сами
                        if let (Some((off, len)), false) = (range, partial) {
                            let start = (off as usize).min(b.len());
                            let end = (start + len as usize).min(b.len());
                            return Ok(b[start..end].to_vec());
                        }
                        return Ok(b.to_vec());
                    }
                    Err(e) => last = e.to_string(),
                }
            }
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

// ───────────────────────────── прогрессивный поток ─────────────────────────────

#[derive(Default)]
struct ProgState {
    data: Vec<u8>,
    total: Option<u64>,
    done: bool,
    error: Option<String>,
}

pub struct ProgBuffer {
    st: Mutex<ProgState>,
    cv: Condvar,
    cancel: AtomicBool,
}

impl ProgBuffer {
    pub fn start(http: reqwest::Client, rt: &Handle, url: String, auth: Option<String>) -> Arc<Self> {
        let buf = Arc::new(Self { st: Mutex::new(ProgState::default()), cv: Condvar::new(), cancel: AtomicBool::new(false) });
        let b = buf.clone();
        rt.spawn(async move { b.download(http, url, auth).await });
        buf
    }

    async fn download(self: Arc<Self>, http: reqwest::Client, url: String, auth: Option<String>) {
        if is_chunked_host(&url) {
            return self.download_chunked(http, url).await;
        }
        let mut failures = 0u64;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return;
            }
            let have = self.st.lock().data.len() as u64;
            let mut req = http.get(&url);
            if let Some(a) = auth_for(&url, &auth) {
                req = req.header(AUTHORIZATION, a);
            }
            if have > 0 {
                req = req.header(RANGE, format!("bytes={have}-"));
            }
            match req.send().await {
                Ok(mut resp) => {
                    let status = resp.status();
                    if status.is_client_error() && status.as_u16() != 429 {
                        self.fail(format!("HTTP {status}"));
                        return;
                    }
                    if status.is_success() {
                        {
                            let mut st = self.st.lock();
                            if have > 0 && status.as_u16() != 206 {
                                // сервер проигнорировал Range — начинаем заново
                                st.data.clear();
                            }
                            if st.total.is_none() {
                                if let Some(len) = resp.content_length() {
                                    st.total = Some(len + if status.as_u16() == 206 { have } else { 0 });
                                }
                            }
                        }
                        loop {
                            if self.cancel.load(Ordering::Relaxed) {
                                return;
                            }
                            match resp.chunk().await {
                                Ok(Some(chunk)) => {
                                    self.st.lock().data.extend_from_slice(&chunk);
                                    self.cv.notify_all();
                                }
                                Ok(None) => {
                                    let mut st = self.st.lock();
                                    st.done = true;
                                    st.total = Some(st.data.len() as u64);
                                    drop(st);
                                    self.cv.notify_all();
                                    return;
                                }
                                Err(e) => {
                                    tracing::warn!("прогрессивная загрузка оборвалась: {e}");
                                    break;
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("ошибка запроса потока: {e}"),
            }
            failures += 1;
            if failures > 5 {
                self.fail("не удалось загрузить поток".into());
                return;
            }
            tokio::time::sleep(Duration::from_millis(500 * failures)).await;
        }
    }

    /// Загрузка кусками через Range. YouTube (googlevideo) режет запрос «весь
    /// файл сразу» примерно до 30 КБ/с, а каждый кусок отдаёт на полной скорости
    /// (так же качает и сам yt-dlp). Первый кусок маленький — звук начинается
    /// сразу, дальше крупнее.
    async fn download_chunked(self: Arc<Self>, http: reqwest::Client, url: String) {
        const FIRST: u64 = 1 << 20;
        const NEXT: u64 = 4 << 20;
        let mut failures = 0u64;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return;
            }
            let (have, total) = {
                let st = self.st.lock();
                (st.data.len() as u64, st.total)
            };
            if total.is_some_and(|t| have >= t) {
                self.st.lock().done = true;
                self.cv.notify_all();
                return;
            }
            let size = if have == 0 { FIRST } else { NEXT };
            let end = total.map_or(have + size, |t| t.min(have + size)) - 1;
            let res = http.get(&url).header(RANGE, format!("bytes={have}-{end}")).send().await;
            let ok = match res {
                Ok(mut resp) => {
                    let status = resp.status();
                    if status.is_client_error() && status.as_u16() != 429 && status.as_u16() != 416 {
                        self.fail(format!("HTTP {status}"));
                        return;
                    }
                    if status.as_u16() == 416 {
                        // запросили за концом файла — значит, всё скачано
                        let mut st = self.st.lock();
                        st.done = true;
                        st.total = Some(st.data.len() as u64);
                        drop(st);
                        self.cv.notify_all();
                        return;
                    }
                    if status.is_success() {
                        // «bytes a-b/total» — полный размер файла
                        let full = resp
                            .headers()
                            .get(reqwest::header::CONTENT_RANGE)
                            .and_then(|v| v.to_str().ok())
                            .and_then(|v| v.rsplit('/').next())
                            .and_then(|v| v.trim().parse::<u64>().ok());
                        {
                            let mut st = self.st.lock();
                            if status.as_u16() != 206 {
                                // Range проигнорирован — пришёл весь файл с начала
                                st.data.clear();
                                st.total = resp.content_length();
                            } else if st.total.is_none() {
                                st.total = full;
                            }
                        }
                        let mut got = true;
                        loop {
                            if self.cancel.load(Ordering::Relaxed) {
                                return;
                            }
                            match resp.chunk().await {
                                Ok(Some(chunk)) => {
                                    self.st.lock().data.extend_from_slice(&chunk);
                                    self.cv.notify_all();
                                }
                                Ok(None) => break,
                                Err(e) => {
                                    tracing::warn!("загрузка куска оборвалась: {e}");
                                    got = false;
                                    break;
                                }
                            }
                        }
                        if status.as_u16() != 206 && got {
                            let mut st = self.st.lock();
                            st.done = true;
                            st.total = Some(st.data.len() as u64);
                            drop(st);
                            self.cv.notify_all();
                            return;
                        }
                        got
                    } else {
                        false
                    }
                }
                Err(e) => {
                    tracing::warn!("ошибка запроса потока: {e}");
                    false
                }
            };
            if ok {
                failures = 0;
                continue;
            }
            failures += 1;
            if failures > 5 {
                self.fail("не удалось загрузить поток".into());
                return;
            }
            tokio::time::sleep(Duration::from_millis(500 * failures)).await;
        }
    }

    fn fail(&self, e: String) {
        self.st.lock().error = Some(e);
        self.cv.notify_all();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cv.notify_all();
    }

    pub fn progress(&self) -> f32 {
        let st = self.st.lock();
        match st.total {
            Some(t) if t > 0 => (st.data.len() as f64 / t as f64).min(1.0) as f32,
            _ if st.done => 1.0,
            _ => 0.0,
        }
    }

    pub fn head(&self, n: usize) -> Vec<u8> {
        let st = self.st.lock();
        st.data[..st.data.len().min(n)].to_vec()
    }

    /// Ждёт, пока в буфере появится хотя бы `n` байт (или поток закончится).
    pub fn wait_bytes(&self, n: usize, abort: &AtomicBool) -> Result<(), String> {
        let mut st = self.st.lock();
        loop {
            if st.data.len() >= n || st.done {
                return Ok(());
            }
            if let Some(e) = &st.error {
                return Err(e.clone());
            }
            if abort.load(Ordering::Relaxed) {
                return Err("прервано".into());
            }
            self.cv.wait_for(&mut st, Duration::from_millis(50));
        }
    }
}

pub struct ProgReader {
    buf: Arc<ProgBuffer>,
    pos: u64,
    abort: Arc<AtomicBool>,
    waiting: Arc<AtomicBool>,
}

impl ProgReader {
    pub fn new(buf: Arc<ProgBuffer>, abort: Arc<AtomicBool>, waiting: Arc<AtomicBool>) -> Self {
        Self { buf, pos: 0, abort, waiting }
    }
}

impl Read for ProgReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        let mut st = self.buf.st.lock();
        loop {
            let len = st.data.len() as u64;
            if self.pos < len {
                let start = self.pos as usize;
                let n = out.len().min(st.data.len() - start);
                out[..n].copy_from_slice(&st.data[start..start + n]);
                self.pos += n as u64;
                return Ok(n);
            }
            if st.done {
                return Ok(0);
            }
            if let Some(e) = &st.error {
                return Err(io::Error::other(e.clone()));
            }
            if self.abort.load(Ordering::Relaxed) {
                return Err(aborted());
            }
            self.waiting.store(true, Ordering::Relaxed);
            self.buf.cv.wait_for(&mut st, Duration::from_millis(50));
            self.waiting.store(false, Ordering::Relaxed);
        }
    }
}

impl Seek for ProgReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let new = match pos {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => {
                let total = self.buf.st.lock().total.ok_or_else(|| io::Error::new(io::ErrorKind::Unsupported, "длина неизвестна"))?;
                total as i64 + d
            }
        };
        if new < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek до начала"));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

impl MediaSource for ProgReader {
    fn is_seekable(&self) -> bool {
        self.buf.st.lock().total.is_some()
    }
    fn byte_len(&self) -> Option<u64> {
        self.buf.st.lock().total
    }
}

// ───────────────────────────────────── HLS ─────────────────────────────────────

struct SegSlot {
    uri: String,
    range: Option<hls::ByteRange>,
    start_ms: u64,
    dur_ms: u64,
    data: Option<Arc<Vec<u8>>>,
    failed: bool,
}

#[derive(Default)]
struct HlsState {
    ready: bool,
    map_uri: Option<String>,
    init: Option<Arc<Vec<u8>>>,
    segs: Vec<SegSlot>,
    container: Option<Container>,
    total_ms: u64,
    bytes: usize,
    error: Option<String>,
}

pub struct HlsBuffer {
    st: Mutex<HlsState>,
    cv: Condvar,
    cancel: AtomicBool,
    want: AtomicUsize,
    notify: Notify,
}

/// Сколько держать в памяти, прежде чем вытеснять уже проигранные сегменты.
const HLS_MEMORY_CAP: usize = 48 * 1024 * 1024;

impl HlsBuffer {
    pub fn start(http: reqwest::Client, rt: &Handle, url: String, auth: Option<String>, read_ahead_ms: u64) -> Arc<Self> {
        let buf = Arc::new(Self {
            st: Mutex::new(HlsState::default()),
            cv: Condvar::new(),
            cancel: AtomicBool::new(false),
            want: AtomicUsize::new(0),
            notify: Notify::new(),
        });
        let b = buf.clone();
        rt.spawn(async move {
            if let Err(e) = b.clone().download(http, url, auth, read_ahead_ms).await {
                tracing::warn!("HLS: {e}");
                b.st.lock().error = Some(e);
                b.cv.notify_all();
            }
        });
        buf
    }

    async fn playlist(http: &reqwest::Client, url: &str, auth: &Option<String>) -> Result<(hls::MediaPlaylist, String), String> {
        let text = String::from_utf8_lossy(&fetch_bytes(http, url, auth, None).await?).into_owned();
        match hls::parse(&text, url)? {
            Playlist::Media(m) => Ok((m, url.to_string())),
            Playlist::Master(vars) => {
                let v = hls::pick_variant(&vars).ok_or("в мастер-плейлисте нет вариантов")?;
                let text = String::from_utf8_lossy(&fetch_bytes(http, &v.uri, auth, None).await?).into_owned();
                match hls::parse(&text, &v.uri)? {
                    Playlist::Media(m) => Ok((m, v.uri.clone())),
                    Playlist::Master(_) => Err("вложенный мастер-плейлист".into()),
                }
            }
        }
    }

    async fn download(self: Arc<Self>, http: reqwest::Client, url: String, auth: Option<String>, read_ahead_ms: u64) -> Result<(), String> {
        let (media, _) = Self::playlist(&http, &url, &auth).await?;
        if media.encrypted {
            return Err("поток защищён DRM — такой трек воспроизвести нельзя".into());
        }
        {
            let mut st = self.st.lock();
            let mut t = 0u64;
            st.segs = media
                .segments
                .iter()
                .map(|s| {
                    let dur = (s.duration * 1000.0).round().max(1.0) as u64;
                    let slot = SegSlot { uri: s.uri.clone(), range: s.range, start_ms: t, dur_ms: dur, data: None, failed: false };
                    t += dur;
                    slot
                })
                .collect();
            st.total_ms = t;
            st.map_uri = media.map.clone();
            if media.map.is_some() {
                st.container = Some(Container::Mp4);
            }
            st.ready = true;
        }
        self.cv.notify_all();

        if let Some(map) = &media.map {
            let init = fetch_bytes(&http, map, &auth, media.map_range).await.map_err(|e| format!("init-сегмент: {e}"))?;
            self.st.lock().init = Some(Arc::new(init));
            self.cv.notify_all();
        }

        let mut consecutive_fail = 0;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Ok(());
            }
            let next = self.next_job(read_ahead_ms);
            let Some((idx, uri, range)) = next else {
                // окно чтения вперёд заполнено — ждём, пока читатель продвинется
                let _ = tokio::time::timeout(Duration::from_millis(1000), self.notify.notified()).await;
                continue;
            };
            match fetch_bytes(&http, &uri, &auth, range).await {
                Ok(raw) => {
                    consecutive_fail = 0;
                    self.store(idx, raw);
                }
                Err(e) => {
                    consecutive_fail += 1;
                    tracing::warn!("HLS сегмент {idx}: {e}");
                    let mut st = self.st.lock();
                    if let Some(s) = st.segs.get_mut(idx) {
                        s.failed = true;
                    }
                    if consecutive_fail >= 4 {
                        st.error = Some(format!("сегменты не загружаются: {e}"));
                    }
                    drop(st);
                    self.cv.notify_all();
                    if consecutive_fail >= 4 {
                        return Ok(());
                    }
                }
            }
        }
    }

    fn next_job(&self, read_ahead_ms: u64) -> Option<(usize, String, Option<hls::ByteRange>)> {
        let st = self.st.lock();
        if !st.ready || (st.map_uri.is_some() && st.init.is_none()) {
            return None;
        }
        let want = self.want.load(Ordering::Relaxed).min(st.segs.len().saturating_sub(1));
        let horizon = st.segs.get(want)?.start_ms.saturating_add(read_ahead_ms.max(15_000));
        st.segs[want..]
            .iter()
            .enumerate()
            .take_while(|(_, s)| s.start_ms <= horizon)
            .find(|(_, s)| s.data.is_none() && !s.failed)
            .map(|(i, s)| (want + i, s.uri.clone(), s.range))
    }

    fn store(&self, idx: usize, raw: Vec<u8>) {
        let mut st = self.st.lock();
        let payload = if is_ts(&raw) {
            match ts::demux_audio(&raw) {
                Some((kind, es)) => {
                    if st.container.is_none() {
                        st.container = Some(match kind {
                            ts::EsKind::Adts => Container::Adts,
                            ts::EsKind::Mpeg => Container::Mp3,
                        });
                    }
                    es
                }
                None => Vec::new(),
            }
        } else {
            let body = strip_id3(&raw);
            if st.container.is_none() {
                st.container = Some(detect(body));
            }
            if body.len() == raw.len() {
                raw
            } else {
                body.to_vec()
            }
        };
        st.bytes += payload.len();
        if let Some(s) = st.segs.get_mut(idx) {
            s.data = Some(Arc::new(payload));
        }
        // длинные миксы: выталкиваем давно проигранные сегменты
        if st.bytes > HLS_MEMORY_CAP {
            let keep_from = self.want.load(Ordering::Relaxed).saturating_sub(6).min(st.segs.len());
            let mut freed = 0;
            for s in st.segs[..keep_from].iter_mut() {
                if let Some(d) = s.data.take() {
                    freed += d.len();
                }
            }
            st.bytes -= freed;
        }
        drop(st);
        self.cv.notify_all();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.notify.notify_one();
        self.cv.notify_all();
    }

    pub fn progress(&self) -> f32 {
        let st = self.st.lock();
        if st.total_ms == 0 {
            return 0.0;
        }
        // доля до первого незагруженного сегмента после текущей позиции
        let want = self.want.load(Ordering::Relaxed);
        let mut end = 0;
        for s in st.segs.iter().skip(want) {
            if s.data.is_none() && !s.failed {
                break;
            }
            end = s.start_ms + s.dur_ms;
        }
        if end == 0 {
            if let Some(s) = st.segs.get(want) {
                end = s.start_ms;
            }
        }
        (end as f64 / st.total_ms as f64).min(1.0) as f32
    }

    /// Ждёт только разбора плейлиста (границы сегментов уже известны).
    pub fn wait_playlist(&self, abort: &AtomicBool) -> Result<u64, String> {
        let mut st = self.st.lock();
        loop {
            if let Some(e) = &st.error {
                return Err(e.clone());
            }
            if st.ready {
                return Ok(st.total_ms);
            }
            if abort.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
                return Err("прервано".into());
            }
            self.cv.wait_for(&mut st, Duration::from_millis(50));
        }
    }

    /// Ждёт плейлист и первый сегмент; возвращает (длительность, контейнер).
    pub fn wait_ready(&self, abort: &AtomicBool) -> Result<(u64, Container), String> {
        let mut st = self.st.lock();
        loop {
            if let Some(e) = &st.error {
                return Err(e.clone());
            }
            if st.ready {
                if let Some(c) = st.container {
                    return Ok((st.total_ms, c));
                }
            }
            if abort.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed) {
                return Err("прервано".into());
            }
            self.cv.wait_for(&mut st, Duration::from_millis(50));
        }
    }

    /// Индекс сегмента, содержащего момент `ms`, и его начало.
    pub fn segment_at(&self, ms: u64) -> (usize, u64) {
        let st = self.st.lock();
        let idx = st
            .segs
            .iter()
            .position(|s| ms < s.start_ms + s.dur_ms)
            .unwrap_or(st.segs.len().saturating_sub(1));
        let start = st.segs.get(idx).map(|s| s.start_ms).unwrap_or(0);
        (idx, start)
    }

    pub fn set_want(&self, idx: usize) {
        if self.want.swap(idx, Ordering::Relaxed) != idx {
            self.notify.notify_one();
        }
    }
}

pub struct HlsReader {
    buf: Arc<HlsBuffer>,
    seg: usize,
    off: usize,
    cur: Option<Arc<Vec<u8>>>,
    init: Option<(Arc<Vec<u8>>, usize)>,
    need_init: bool,
    pos: u64,
    abort: Arc<AtomicBool>,
    waiting: Arc<AtomicBool>,
}

impl HlsReader {
    pub fn new(buf: Arc<HlsBuffer>, seg: usize, abort: Arc<AtomicBool>, waiting: Arc<AtomicBool>) -> Self {
        let need_init = buf.st.lock().map_uri.is_some();
        buf.set_want(seg);
        Self { buf, seg, off: 0, cur: None, init: None, need_init, pos: 0, abort, waiting }
    }

    fn wait<'a>(&self, st: &mut parking_lot::MutexGuard<'a, HlsState>) -> io::Result<()> {
        if let Some(e) = &st.error {
            return Err(io::Error::other(e.clone()));
        }
        if self.abort.load(Ordering::Relaxed) {
            return Err(aborted());
        }
        self.waiting.store(true, Ordering::Relaxed);
        self.buf.cv.wait_for(st, Duration::from_millis(50));
        self.waiting.store(false, Ordering::Relaxed);
        Ok(())
    }
}

impl Read for HlsReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            if self.need_init {
                let mut st = self.buf.st.lock();
                match st.init.clone() {
                    Some(d) => {
                        self.init = Some((d, 0));
                        self.need_init = false;
                    }
                    None => {
                        self.wait(&mut st)?;
                        continue;
                    }
                }
            }
            if let Some((data, off)) = &mut self.init {
                if *off < data.len() {
                    let n = out.len().min(data.len() - *off);
                    out[..n].copy_from_slice(&data[*off..*off + n]);
                    *off += n;
                    self.pos += n as u64;
                    return Ok(n);
                }
                self.init = None;
            }
            if let Some(cur) = &self.cur {
                if self.off < cur.len() {
                    let n = out.len().min(cur.len() - self.off);
                    out[..n].copy_from_slice(&cur[self.off..self.off + n]);
                    self.off += n;
                    self.pos += n as u64;
                    return Ok(n);
                }
                self.cur = None;
                self.seg += 1;
                self.off = 0;
                self.buf.set_want(self.seg);
            }
            let mut st = self.buf.st.lock();
            if self.seg >= st.segs.len() {
                if st.ready {
                    return Ok(0);
                }
                self.wait(&mut st)?;
                continue;
            }
            let slot = &st.segs[self.seg];
            if let Some(d) = slot.data.clone() {
                self.cur = Some(d);
                continue;
            }
            if slot.failed {
                // сегмент не скачался — пропускаем, лучше короткий провал, чем стоп
                self.seg += 1;
                drop(st);
                self.buf.set_want(self.seg);
                continue;
            }
            self.wait(&mut st)?;
        }
    }
}

impl Seek for HlsReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::Current(0) => Ok(self.pos),
            _ => Err(io::Error::new(io::ErrorKind::Unsupported, "HLS-поток читается последовательно")),
        }
    }
}

impl MediaSource for HlsReader {
    fn is_seekable(&self) -> bool {
        false
    }
    fn byte_len(&self) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_containers() {
        assert_eq!(detect(&[0, 0, 0, 0x18, b'f', b't', b'y', b'p', 0, 0]), Container::Mp4);
        assert_eq!(detect(&[0xff, 0xf1, 0x50, 0x80]), Container::Adts);
        assert_eq!(detect(&[0xff, 0xfb, 0x90, 0x64]), Container::Mp3);
        let mut id3 = b"ID3\x04\x00\x00\x00\x00\x00\x02ab".to_vec();
        id3.extend_from_slice(&[0xff, 0xfb, 0x90, 0x64]);
        assert_eq!(detect(&id3), Container::Mp3);
        assert_eq!(detect(b"OggS\x00\x02"), Container::Ogg);
    }

    #[test]
    fn auth_only_for_soundcloud_hosts() {
        let a = Some("OAuth x".to_string());
        assert!(auth_for("https://api.soundcloud.com/tracks/1/streams", &a).is_some());
        assert!(auth_for("https://playback.media-streaming.soundcloud.cloud/x.m3u8", &a).is_some());
        assert!(auth_for("https://evil.example.com/soundcloud.com", &a).is_none());
    }
}

/// Хосты, которые режут скорость запроса «весь файл сразу» и отдают быстро
/// только куски (YouTube / googlevideo).
fn is_chunked_host(url: &str) -> bool {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|h| h.ends_with(".googlevideo.com") || h.ends_with(".youtube.com"))
}

#[cfg(test)]
mod chunk_tests {
    use super::is_chunked_host;

    #[test]
    fn youtube_streams_are_chunked() {
        assert!(is_chunked_host("https://rr3---sn-ivgt1vou2pm-2v1l.googlevideo.com/videoplayback?expire=1&itag=140"));
        assert!(!is_chunked_host("https://cf-media.sndcdn.com/abc.mp3"));
        assert!(!is_chunked_host("not a url"));
    }

    /// Живая проверка: `NEBULA_TEST_YT_URL=<ссылка googlevideo> cargo test live_chunked -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_chunked_youtube_download() {
        let Ok(url) = std::env::var("NEBULA_TEST_YT_URL") else { return };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let http = reqwest::Client::builder().user_agent("NebulaPlayer/test").build().unwrap();
        let started = std::time::Instant::now();
        let buf = super::ProgBuffer::start(http, rt.handle(), url, None);
        while buf.progress() < 1.0 && started.elapsed() < std::time::Duration::from_secs(30) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let secs = started.elapsed().as_secs_f64();
        let head = buf.head(12);
        let size = buf.st.lock().data.len();
        println!("скачано {size} байт за {secs:.2} с ({:.1} МБ/с)", size as f64 / secs / 1e6);
        assert!(buf.progress() >= 1.0, "файл не докачался за 30 с");
        assert_eq!(&head[4..8], b"ftyp", "это не MP4");
        assert!(size as f64 / secs > 1e6, "медленнее 1 МБ/с");
    }
}
