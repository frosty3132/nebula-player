//! Обработчик `/media/*` внутри WebView: картинки и файлы отдаются из Rust.
//!
//! * `/media/img?u=<url>`   — обложки/аватары: дисковый кэш (LRU по размеру), сеть при промахе.
//! * `/media/cover?p=<путь>` — встроенная обложка локального трека.
//! * `/media/file?p=<путь>`  — фон пользователя (картинка/GIF/видео) с поддержкой Range.
//!
//! Плюс извлечение «акцентного» цвета из обложки для динамической темы.

use crate::paths::{hash_key, paths};
use dioxus_desktop::wry::http::{header, Request, Response, StatusCode};
use dioxus_desktop::wry::RequestAsyncResponder;
use parking_lot::Mutex;
use reqwest::Url;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

pub fn sniff_mime(d: &[u8]) -> &'static str {
    if d.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if d.starts_with(b"\x89PNG") {
        "image/png"
    } else if d.starts_with(b"GIF8") {
        "image/gif"
    } else if d.len() > 12 && &d[..4] == b"RIFF" && &d[8..12] == b"WEBP" {
        "image/webp"
    } else if d.starts_with(b"BM") {
        "image/bmp"
    } else if d.starts_with(&[0x00, 0x00, 0x01, 0x00]) || d.starts_with(&[0x00, 0x00, 0x02, 0x00]) {
        "image/x-icon"
    } else if d.starts_with(b"II*\0") || d.starts_with(b"MM\0*") {
        "image/tiff"
    } else if d.len() >= 12 && &d[4..8] == b"ftyp" {
        match &d[8..12] {
            b"avif" | b"avis" => "image/avif",
            b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" => "image/heif",
            _ => "application/octet-stream",
        }
    } else if std::str::from_utf8(&d[..d.len().min(512)])
        .ok()
        .is_some_and(|s| {
            let t = s.trim_start_matches('\u{feff}').trim_start();
            t.starts_with("<svg") || (t.starts_with("<?xml") && t.contains("<svg"))
        })
    {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn mime_by_ext(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg" | "jfif" | "pjpeg" | "pjp") => "image/jpeg",
        Some("png" | "apng") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("heic" | "heif") => "image/heif",
        Some("bmp") => "image/bmp",
        Some("ico" | "cur") => "image/x-icon",
        Some("tif" | "tiff") => "image/tiff",
        Some("svg") => "image/svg+xml",
        Some("mp4" | "m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        _ => "",
    }
}

/// Маршрут (`/img`, `/cover`, `/file`) и параметры запроса к `/media/*`.
///
/// Берём только путь и строку запроса: схема и хост зависят от платформы.
/// На Windows WebView2 запрашивает `http://dioxus.index.html/media/...`, но
/// wry перед вызовом обработчика возвращает адрес к виду
/// `dioxus://index.html/media/...` — раньше такой адрес разбирался неверно,
/// и все обложки получали 404.
fn parse_media_uri(uri: &dioxus_desktop::wry::http::Uri) -> Option<(String, HashMap<String, String>)> {
    let pq = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let url = Url::parse(&format!("http://local{pq}")).ok()?;
    let q = url.query_pairs().into_owned().collect();
    Some((url.path().trim_start_matches("/media").to_string(), q))
}

fn respond_status(responder: RequestAsyncResponder, status: StatusCode) {
    responder.respond(Response::builder().status(status).body(Vec::new()).unwrap());
}

fn respond_bytes(responder: RequestAsyncResponder, data: Vec<u8>, mime: &str, cache: bool) {
    let mut b = Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, mime);
    if cache {
        b = b.header(header::CACHE_CONTROL, "public, max-age=31536000, immutable");
    }
    responder.respond(b.body(data).unwrap());
}

pub struct Media {
    http: reqwest::Client,
    accents: Mutex<HashMap<String, Option<String>>>,
    downloads: AtomicU32,
    limit_mb: AtomicU32,
}

impl Media {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http, accents: Mutex::new(HashMap::new()), downloads: AtomicU32::new(0), limit_mb: AtomicU32::new(400) }
    }

    pub fn set_limit_mb(&self, mb: u32) {
        self.limit_mb.store(mb.max(32), Ordering::Relaxed);
    }

    fn cache_file(url: &str) -> PathBuf {
        let h = hash_key(url);
        paths().images.join(&h[..2]).join(h)
    }

    /// Байты картинки: из дискового кэша или из сети (с записью в кэш).
    pub async fn image_bytes(&self, url: &str) -> Option<Vec<u8>> {
        let file = Self::cache_file(url);
        if let Ok(d) = tokio::fs::read(&file).await {
            return Some(d);
        }
        let parsed = Url::parse(url).ok()?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return None;
        }
        let resp = self.http.get(parsed).timeout(Duration::from_secs(20)).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let data = resp.bytes().await.ok()?.to_vec();
        if let Some(dir) = file.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        let tmp = file.with_extension("part");
        if tokio::fs::write(&tmp, &data).await.is_ok() {
            let _ = tokio::fs::rename(&tmp, &file).await;
        }
        if self.downloads.fetch_add(1, Ordering::Relaxed) % 64 == 63 {
            let limit = self.limit_mb.load(Ordering::Relaxed);
            tokio::task::spawn_blocking(move || trim_cache(limit));
        }
        Some(data)
    }

    pub fn handle(&'static self, req: Request<Vec<u8>>, responder: RequestAsyncResponder) {
        let range = req.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
        let Some((route, q)) = parse_media_uri(req.uri()) else {
            return respond_status(responder, StatusCode::BAD_REQUEST);
        };

        crate::services::svc().rt.spawn(async move {
            match route.as_str() {
                "/img" => {
                    let Some(u) = q.get("u") else { return respond_status(responder, StatusCode::BAD_REQUEST) };
                    match self.image_bytes(u).await {
                        Some(d) => {
                            let mime = sniff_mime(&d);
                            respond_bytes(responder, d, mime, true)
                        }
                        None => respond_status(responder, StatusCode::NOT_FOUND),
                    }
                }
                "/cover" => {
                    let Some(p) = q.get("p").cloned() else { return respond_status(responder, StatusCode::BAD_REQUEST) };
                    match tokio::task::spawn_blocking(move || crate::library::local_cover(&p)).await.ok().flatten() {
                        Some((d, mime)) => respond_bytes(responder, d, mime, true),
                        None => respond_status(responder, StatusCode::NOT_FOUND),
                    }
                }
                "/file" => {
                    let Some(p) = q.get("p").cloned() else { return respond_status(responder, StatusCode::BAD_REQUEST) };
                    let res = tokio::task::spawn_blocking(move || serve_file(Path::new(&p), range.as_deref())).await;
                    match res {
                        Ok(Some(resp)) => responder.respond(resp),
                        _ => respond_status(responder, StatusCode::NOT_FOUND),
                    }
                }
                _ => respond_status(responder, StatusCode::NOT_FOUND),
            }
        });
    }

    /// Акцентный цвет обложки (#rrggbb) — для динамической подсветки интерфейса.
    pub async fn accent_for(&self, url: &str) -> Option<String> {
        if let Some(hit) = self.accents.lock().get(url).cloned() {
            return hit;
        }
        let bytes = if let Some(rest) = url.strip_prefix("file:") {
            let p = rest.to_string();
            tokio::task::spawn_blocking(move || crate::library::local_cover(&p).map(|(d, _)| d)).await.ok().flatten()
        } else {
            self.image_bytes(url).await
        }?;
        let res = tokio::task::spawn_blocking(move || dominant_color(&bytes)).await.ok().flatten();
        self.accents.lock().insert(url.to_string(), res.clone());
        res
    }
}

/// Отдаём только медиафайлы (картинки и видео) — WebView не получит доступ к прочим файлам.
fn serve_file(path: &Path, range: Option<&str>) -> Option<Response<Vec<u8>>> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut mime = mime_by_ext(path);
    if mime.is_empty() {
        let mut head = [0u8; 512];
        let n = f.read(&mut head).ok()?;
        let sniffed = sniff_mime(&head[..n]);
        if sniffed == "application/octet-stream" {
            return None;
        }
        mime = sniffed;
        f.seek(SeekFrom::Start(0)).ok()?;
    }
    let len = f.metadata().ok()?.len();
    const CHUNK: u64 = 4 * 1024 * 1024;

    let parsed = range.and_then(|r| r.strip_prefix("bytes=")).and_then(|r| {
        let (a, b) = r.split_once('-')?;
        let start: u64 = a.trim().parse().ok()?;
        let end: u64 = if b.trim().is_empty() { len.saturating_sub(1) } else { b.trim().parse().ok()? };
        (start < len).then_some((start, end.min(len - 1)))
    });

    if let Some((start, end)) = parsed {
        let end = end.min(start + CHUNK - 1);
        let mut buf = vec![0u8; (end - start + 1) as usize];
        f.seek(SeekFrom::Start(start)).ok()?;
        f.read_exact(&mut buf).ok()?;
        return Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"))
            .body(buf)
            .ok();
    }

    if mime.starts_with("video/") && len > CHUNK {
        // видео без Range: отдаём первый кусок, плеер дозапросит остальное
        let mut buf = vec![0u8; CHUNK as usize];
        f.read_exact(&mut buf).ok()?;
        return Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_TYPE, mime)
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_RANGE, format!("bytes 0-{}/{len}", CHUNK - 1))
            .body(buf)
            .ok();
    }

    let mut buf = Vec::with_capacity(len as usize);
    f.read_to_end(&mut buf).ok()?;
    Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, mime).header(header::ACCEPT_RANGES, "bytes").body(buf).ok()
}

/// Удаляем самые старые файлы, пока кэш не уложится в 80% лимита.
pub fn trim_cache(limit_mb: u32) {
    let limit = limit_mb as u64 * 1024 * 1024;
    let mut files = Vec::new();
    let mut total = 0u64;
    if let Ok(dirs) = std::fs::read_dir(&paths().images) {
        for d in dirs.flatten() {
            if let Ok(inner) = std::fs::read_dir(d.path()) {
                for f in inner.flatten() {
                    if let Ok(m) = f.metadata() {
                        if m.is_file() {
                            total += m.len();
                            files.push((m.modified().ok(), m.len(), f.path()));
                        }
                    }
                }
            }
        }
    }
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.0);
    let target = limit * 8 / 10;
    for (_, size, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= size;
        }
    }
}

pub fn cache_size_mb() -> f64 {
    fn dir_size(p: &Path) -> u64 {
        std::fs::read_dir(p)
            .map(|rd| {
                rd.flatten()
                    .map(|e| match e.metadata() {
                        Ok(m) if m.is_dir() => dir_size(&e.path()),
                        Ok(m) => m.len(),
                        Err(_) => 0,
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    let p = paths();
    (dir_size(&p.images) + dir_size(&p.covers) + dir_size(&p.lyrics)) as f64 / (1024.0 * 1024.0)
}

pub fn clear_cache() {
    let p = paths();
    for d in [&p.images, &p.covers, &p.lyrics] {
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::create_dir_all(d);
    }
}

fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if (max - min).abs() < 1e-6 {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h, s, l)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    if s <= 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

/// Насыщенный «средний» цвет обложки; для серых обложек — None.
pub fn dominant_color(bytes: &[u8]) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.thumbnail(40, 40).to_rgb8();
    let (mut sr, mut sg, mut sb, mut sw) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in small.pixels() {
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let (_, s, l) = rgb_to_hsl(r, g, b);
        let w = s * s * (1.0 - (l - 0.55).abs() * 1.7).max(0.0);
        sr += r * w;
        sg += g * w;
        sb += b * w;
        sw += w;
    }
    if sw < 0.6 {
        return None;
    }
    let (h, s, l) = rgb_to_hsl(sr / sw, sg / sw, sb / sw);
    let (r, g, b) = hsl_to_rgb(h, s.max(0.6), l.clamp(0.55, 0.68));
    Some(format!("#{:02x}{:02x}{:02x}", (r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_uri_in_both_platform_forms() {
        let art = "https://i1.sndcdn.com/artworks-abc-t300x300.jpg";
        let enc = urlencoding::encode(art);
        for raw in [
            format!("dioxus://index.html/media/img?u={enc}"),
            format!("http://dioxus.index.html/media/img?u={enc}"),
            format!("/media/img?u={enc}"),
        ] {
            let uri: dioxus_desktop::wry::http::Uri = raw.parse().unwrap();
            let (route, q) = parse_media_uri(&uri).unwrap();
            assert_eq!(route, "/img", "{raw}");
            assert_eq!(q.get("u").map(String::as_str), Some(art), "{raw}");
        }
        let uri: dioxus_desktop::wry::http::Uri = "dioxus://index.html/media/cover?p=C%3A%5CMusic%5Ca%20b.mp3".parse().unwrap();
        let (route, q) = parse_media_uri(&uri).unwrap();
        assert_eq!(route, "/cover");
        assert_eq!(q.get("p").map(String::as_str), Some(r"C:\Music\a b.mp3"));
    }

    #[test]
    fn mime_sniffing() {
        assert_eq!(sniff_mime(&[0xff, 0xd8, 0xff, 0xe0]), "image/jpeg");
        assert_eq!(sniff_mime(b"\x89PNG\r\n"), "image/png");
        assert_eq!(sniff_mime(b"GIF89a"), "image/gif");
    }

    #[test]
    fn hsl_roundtrip() {
        let (h, s, l) = rgb_to_hsl(0.545, 0.36, 0.965);
        let (r, g, b) = hsl_to_rgb(h, s, l);
        assert!((r - 0.545).abs() < 0.01 && (g - 0.36).abs() < 0.01 && (b - 0.965).abs() < 0.01);
    }

    #[test]
    fn dominant_color_of_solid_png() {
        let mut img = image::RgbImage::new(8, 8);
        for p in img.pixels_mut() {
            *p = image::Rgb([200, 40, 120]);
        }
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        let c = dominant_color(out.get_ref()).unwrap();
        assert!(c.starts_with('#') && c.len() == 7, "{c}");
    }
}
