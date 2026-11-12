//! Скачивание трека в файл: тот же поток, что и для воспроизведения
//! (файл, прогрессивный HTTP или HLS), собирается целиком в один файл.
//! HLS: из мастер-плейлиста берётся лучший вариант, сегменты склеиваются
//! (fMP4 — с init-сегментом в .m4a, MPEG-TS — демультиплексируется в .aac/.mp3).

use super::player::MediaSpec;
use super::source::{detect, fetch_bytes, Container};
use super::{hls, ts};

/// Скачать поток целиком: байты и расширение файла.
pub async fn fetch_track(http: &reqwest::Client, spec: &MediaSpec, mut progress: impl FnMut(f32)) -> Result<(Vec<u8>, &'static str), String> {
    match spec {
        MediaSpec::File(p) => {
            let data = tokio::fs::read(p).await.map_err(|e| format!("не удалось прочитать файл: {e}"))?;
            let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
            let ext: &'static str = match ext.as_deref() {
                Some("mp3") => "mp3",
                Some("flac") => "flac",
                Some("m4a") => "m4a",
                Some("ogg") => "ogg",
                Some("wav") => "wav",
                Some("aac") => "aac",
                Some("opus") => "opus",
                _ => detect(&data).ext().unwrap_or("bin"),
            };
            progress(1.0);
            Ok((data, ext))
        }
        MediaSpec::Http { url, auth } => {
            // кусками по 4 МБ: YouTube медленно отдаёт файл целиком одним запросом
            let mut data = Vec::new();
            let chunk = 4u64 << 20;
            loop {
                let part = fetch_bytes(http, url, auth, Some((data.len() as u64, chunk))).await?;
                let n = part.len() as u64;
                data.extend_from_slice(&part);
                progress((data.len() as f32 / 8_000_000.0).min(0.95));
                if n < chunk {
                    break;
                }
            }
            if data.is_empty() {
                return Err("пустой ответ сервера".into());
            }
            let ext = match detect(&data) {
                Container::Mp4 => "m4a",
                c => c.ext().unwrap_or("webm"),
            };
            progress(1.0);
            Ok((data, ext))
        }
        MediaSpec::Hls { url, auth } => {
            let text = String::from_utf8_lossy(&fetch_bytes(http, url, auth, None).await?).to_string();
            let mut media_url = url.clone();
            let mut pl = hls::parse(&text, url)?;
            if let hls::Playlist::Master(vars) = &pl {
                let best = vars.iter().max_by_key(|v| v.bandwidth).ok_or("пустой мастер-плейлист")?;
                media_url = best.uri.clone();
                let t = String::from_utf8_lossy(&fetch_bytes(http, &media_url, auth, None).await?).to_string();
                pl = hls::parse(&t, &media_url)?;
            }
            let hls::Playlist::Media(m) = pl else { return Err("неожиданный плейлист".into()) };
            if m.encrypted {
                return Err("поток зашифрован — его нельзя сохранить".into());
            }
            let mut data = Vec::new();
            if let Some(map) = &m.map {
                data.extend_from_slice(&fetch_bytes(http, map, auth, m.map_range).await?);
            }
            let total = m.segments.len().max(1);
            for (i, seg) in m.segments.iter().enumerate() {
                data.extend_from_slice(&fetch_bytes(http, &seg.uri, auth, seg.range).await?);
                progress((i + 1) as f32 / total as f32);
            }
            // MPEG-TS склеивать в файл бессмысленно — достаём сам звук
            if m.map.is_none() && data.first() == Some(&0x47) {
                if let Some((kind, es)) = ts::demux_audio(&data) {
                    let ext = if kind == ts::EsKind::Adts { "aac" } else { "mp3" };
                    return Ok((es, ext));
                }
            }
            let ext = match detect(&data) {
                Container::Mp4 => "m4a",
                c => c.ext().unwrap_or(if m.map.is_some() { "m4a" } else { "mp3" }),
            };
            let _ = media_url;
            Ok((data, ext))
        }
    }
}

/// Имя файла без запрещённых в Windows символов: «Исполнитель — Название».
pub fn file_name(artist: &str, title: &str, ext: &str) -> String {
    let base = if artist.trim().is_empty() { title.trim().to_string() } else { format!("{} — {}", artist.trim(), title.trim()) };
    let clean: String = base.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    let clean = clean.trim().trim_end_matches('.').to_string();
    let clean = if clean.is_empty() { "track".to_string() } else { clean.chars().take(150).collect() };
    format!("{clean}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn file_name_strips_forbidden_chars() {
        assert_eq!(file_name("AC/DC", "Back: In Black?", "mp3"), "AC_DC — Back_ In Black_.mp3");
        assert_eq!(file_name("", "  ", "m4a"), "track.m4a");
    }
}
