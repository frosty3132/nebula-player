//! Минимальный парсер HLS-плейлистов (RFC 8216) — ровно то, что нужно для
//! аудио-VOD: мастер-плейлисты с вариантами, медиаплейлисты, EXT-X-MAP (fMP4/CMAF),
//! детект шифрования. Относительные URI разрешаются от адреса плейлиста.

use reqwest::Url;

#[derive(Debug, Clone, PartialEq)]
pub struct Variant {
    pub uri: String,
    pub bandwidth: u64,
    pub codecs: Option<String>,
}

/// Диапазон байтов (смещение, длина) из `EXT-X-BYTERANGE`.
pub type ByteRange = (u64, u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub uri: String,
    pub duration: f64,
    pub range: Option<ByteRange>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MediaPlaylist {
    pub segments: Vec<Segment>,
    pub map: Option<String>,
    pub map_range: Option<ByteRange>,
    pub encrypted: bool,
    pub ended: bool,
}

/// `<длина>[@<смещение>]`; без смещения диапазон продолжает предыдущий.
fn parse_range(s: &str, prev_end: u64) -> Option<ByteRange> {
    let s = s.trim().trim_matches('"');
    let (len, off) = match s.split_once('@') {
        Some((l, o)) => (l.trim().parse().ok()?, o.trim().parse().ok()?),
        None => (s.parse().ok()?, prev_end),
    };
    Some((off, len))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master(Vec<Variant>),
    Media(MediaPlaylist),
}

fn resolve(base: &Url, uri: &str) -> String {
    base.join(uri.trim()).map(|u| u.to_string()).unwrap_or_else(|_| uri.trim().to_string())
}

/// Разбор списка атрибутов `KEY=VALUE,KEY="quoted,value"`.
fn attributes(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut key = String::new();
    let mut val = String::new();
    let mut in_key = true;
    let mut quoted = false;
    for ch in s.chars() {
        match ch {
            '"' => quoted = !quoted,
            '=' if in_key => in_key = false,
            ',' if !quoted => {
                if !key.is_empty() {
                    out.push((key.trim().to_ascii_uppercase(), val.trim().to_string()));
                }
                key.clear();
                val.clear();
                in_key = true;
            }
            _ => {
                if in_key {
                    key.push(ch)
                } else {
                    val.push(ch)
                }
            }
        }
    }
    if !key.is_empty() {
        out.push((key.trim().to_ascii_uppercase(), val.trim().to_string()));
    }
    out
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

pub fn parse(text: &str, base_url: &str) -> Result<Playlist, String> {
    let base = Url::parse(base_url).map_err(|e| format!("плохой URL плейлиста: {e}"))?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    match lines.next() {
        Some(first) if first.starts_with("#EXTM3U") => {}
        _ => return Err("это не HLS-плейлист (#EXTM3U не найден)".into()),
    }

    let mut variants = Vec::new();
    let mut media = MediaPlaylist::default();
    let mut pending_variant: Option<(u64, Option<String>)> = None;
    let mut pending_duration: Option<f64> = None;
    let mut pending_range: Option<ByteRange> = None;
    let mut range_end: u64 = 0;

    for line in lines {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let a = attributes(rest);
            let bw = attr(&a, "BANDWIDTH").and_then(|v| v.parse().ok()).unwrap_or(0);
            let codecs = attr(&a, "CODECS").map(str::to_string);
            pending_variant = Some((bw, codecs));
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            let d = rest.split(',').next().unwrap_or("0").trim();
            pending_duration = Some(d.parse().unwrap_or(0.0));
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attributes(rest);
            if let Some(uri) = attr(&a, "URI") {
                media.map = Some(resolve(&base, uri));
                media.map_range = attr(&a, "BYTERANGE").and_then(|r| parse_range(r, 0));
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            pending_range = parse_range(rest, range_end);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attributes(rest);
            let method = attr(&a, "METHOD").unwrap_or("NONE");
            if !method.eq_ignore_ascii_case("NONE") {
                media.encrypted = true;
            }
        } else if line.starts_with("#EXT-X-ENDLIST") {
            media.ended = true;
        } else if line.starts_with('#') {
            // прочие теги нам не нужны
        } else if let Some((bandwidth, codecs)) = pending_variant.take() {
            variants.push(Variant { uri: resolve(&base, line), bandwidth, codecs });
        } else if let Some(duration) = pending_duration.take() {
            let range = pending_range.take();
            if let Some((off, len)) = range {
                range_end = off + len;
            }
            media.segments.push(Segment { uri: resolve(&base, line), duration, range });
        }
    }

    if !variants.is_empty() {
        Ok(Playlist::Master(variants))
    } else if !media.segments.is_empty() {
        Ok(Playlist::Media(media))
    } else {
        Err("в плейлисте нет сегментов".into())
    }
}

/// Выбор лучшего поддерживаемого варианта из мастер-плейлиста.
pub fn pick_variant(variants: &[Variant]) -> Option<&Variant> {
    let supported = |v: &&Variant| match &v.codecs {
        None => true,
        Some(c) => {
            let c = c.to_ascii_lowercase();
            c.contains("mp4a.40.2") || c.contains("mp4a.40.29") || c.contains("mp4a.40.5")
                || c.contains("mp4a.40.34") || c.contains("mp4a.69") || c.contains("mp4a.6b")
                || c.contains("mp3") || c == "mp4a"
        }
    };
    variants
        .iter()
        .filter(supported)
        .max_by_key(|v| v.bandwidth)
        .or_else(|| variants.iter().max_by_key(|v| v.bandwidth))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fmp4_media_playlist() {
        let text = "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"init.mp4?sig=1\"\n#EXTINF:9.985,\nseg0.m4s?sig=2\n#EXTINF:10.0,\nseg1.m4s?sig=3\n#EXT-X-ENDLIST\n";
        let p = parse(text, "https://cdn.example/a/b/playlist.m3u8?token=x").unwrap();
        let Playlist::Media(m) = p else { panic!("expected media") };
        assert_eq!(m.map.as_deref(), Some("https://cdn.example/a/b/init.mp4?sig=1"));
        assert_eq!(m.segments.len(), 2);
        assert_eq!(m.segments[1].uri, "https://cdn.example/a/b/seg1.m4s?sig=3");
        assert!((m.segments[0].duration - 9.985).abs() < 1e-9);
        assert!(m.ended && !m.encrypted);
    }

    #[test]
    fn parses_master_and_picks_best_audio() {
        let text = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=96000,CODECS=\"mp4a.40.2\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=160000,CODECS=\"mp4a.40.2\"\nhigh.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=320000,CODECS=\"opus\"\nopus.m3u8\n";
        let Playlist::Master(v) = parse(text, "https://x.test/m.m3u8").unwrap() else { panic!() };
        assert_eq!(pick_variant(&v).unwrap().uri, "https://x.test/high.m3u8");
    }

    #[test]
    fn parses_byte_ranges() {
        let text = "#EXTM3U\n#EXT-X-MAP:URI=\"main.mp4\",BYTERANGE=\"616@0\"\n#EXTINF:5.99,\n#EXT-X-BYTERANGE:121090@616\nmain.mp4\n#EXTINF:5.99,\n#EXT-X-BYTERANGE:1000\nmain.mp4\n";
        let Playlist::Media(m) = parse(text, "https://x.test/a/prog.m3u8").unwrap() else { panic!() };
        assert_eq!(m.map_range, Some((0, 616)));
        assert_eq!(m.segments[0].range, Some((616, 121090)));
        assert_eq!(m.segments[1].range, Some((121706, 1000)));
    }

    #[test]
    fn detects_encryption() {
        let text = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://k\"\n#EXTINF:5,\na.ts\n";
        let Playlist::Media(m) = parse(text, "https://x.test/p.m3u8").unwrap() else { panic!() };
        assert!(m.encrypted);
    }
}
