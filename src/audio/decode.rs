//! Поток декодера: открывает источник, декодирует symphonia, приводит к стерео,
//! ресемплирует под частоту устройства и пишет в кольцевой буфер.
//! Перемотка = переоткрытие источника с нужной позиции (для HLS — с нужного сегмента)
//! и точный добор: лишние кадры до целевой позиции отбрасываются.

use super::output::RingCtl;
use super::resample::Resampler;
use super::source::{is_aborted, Container, HlsBuffer, HlsReader, ProgBuffer, ProgReader};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

#[derive(Clone)]
pub enum SourceHandle {
    File(PathBuf),
    Prog(Arc<ProgBuffer>),
    Hls(Arc<HlsBuffer>),
}

impl SourceHandle {
    pub fn cancel(&self) {
        match self {
            SourceHandle::File(_) => {}
            SourceHandle::Prog(b) => b.cancel(),
            SourceHandle::Hls(b) => b.cancel(),
        }
    }

    pub fn progress(&self) -> f32 {
        match self {
            SourceHandle::File(_) => 1.0,
            SourceHandle::Prog(b) => b.progress(),
            SourceHandle::Hls(b) => b.progress(),
        }
    }
}

/// Управление одной сессией воспроизведения (один трек).
#[derive(Default)]
pub struct SessionCtl {
    pub stop: AtomicBool,
    pub abort: Arc<AtomicBool>,
    pub waiting: Arc<AtomicBool>,
    pub seek_seq: AtomicU64,
    pub seek_ms: AtomicU64,
    pub seeking: AtomicBool,
    pub ready: AtomicBool,
    pub eof: AtomicBool,
    pub end_frame: AtomicU64,
    pub duration_ms: AtomicU64,
    pub base_ms: AtomicU64,
    pub start_frame: AtomicU64,
    pub error: Mutex<Option<String>>,
}

impl SessionCtl {
    pub fn request_seek(&self, ms: u64) {
        self.seek_ms.store(ms, Ordering::Release);
        self.seeking.store(true, Ordering::Release);
        self.seek_seq.fetch_add(1, Ordering::AcqRel);
        self.abort.store(true, Ordering::Release);
    }

    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.abort.store(true, Ordering::Release);
    }
}

pub struct DecodeCtx {
    pub source: SourceHandle,
    pub ctl: Arc<SessionCtl>,
    pub ring: Arc<RingCtl>,
    pub start_ms: u64,
    pub duration_hint_ms: u64,
}

enum Flow {
    Stop,
    Seek(u64),
}

struct Opened {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    /// Сколько миллисекунд от начала открытого фрагмента нужно отбросить.
    skip_ms: f64,
    duration_ms: u64,
}

pub fn spawn(ctx: DecodeCtx) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("nebula-decode".into())
        .spawn(move || {
            let ctl = ctx.ctl.clone();
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&ctx)));
            let err = match res {
                Ok(Ok(())) => None,
                Ok(Err(e)) => Some(e),
                Err(p) => Some(format!(
                    "декодер упал: {}",
                    p.downcast_ref::<&str>().copied().or_else(|| p.downcast_ref::<String>().map(|s| s.as_str())).unwrap_or("?")
                )),
            };
            if let Some(e) = err {
                if !ctl.stop.load(Ordering::Acquire) {
                    tracing::warn!("декодер: {e}");
                    *ctl.error.lock() = Some(e);
                }
            }
        })
        .expect("не удалось запустить поток декодера")
}

fn check(ctx: &DecodeCtx, seen: &mut u64) -> Option<Flow> {
    if ctx.ctl.stop.load(Ordering::Acquire) {
        return Some(Flow::Stop);
    }
    let s = ctx.ctl.seek_seq.load(Ordering::Acquire);
    if s != *seen {
        *seen = s;
        return Some(Flow::Seek(ctx.ctl.seek_ms.load(Ordering::Acquire)));
    }
    None
}

fn run(ctx: &DecodeCtx) -> Result<(), String> {
    let ctl = &ctx.ctl;
    let mut seen = ctl.seek_seq.load(Ordering::Acquire);
    let mut target_ms = ctx.start_ms;
    let mut sbuf: Option<SampleBuffer<f32>> = None;
    let mut stereo: Vec<[f32; 2]> = Vec::with_capacity(8192);
    let mut out: Vec<f32> = Vec::with_capacity(16384);

    'open: loop {
        if let Some(flow) = check(ctx, &mut seen) {
            match flow {
                Flow::Stop => return Ok(()),
                Flow::Seek(ms) => target_ms = ms,
            }
        }
        ctl.abort.store(false, Ordering::Release);

        let mut opened = match open_at(ctx, target_ms) {
            Ok(o) => o,
            Err(e) => {
                // прерывание открытия из-за новой перемотки/остановки — не ошибка
                if ctl.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                if ctl.seek_seq.load(Ordering::Acquire) != seen {
                    continue 'open;
                }
                return Err(e);
            }
        };
        if opened.duration_ms > 0 {
            ctl.duration_ms.store(opened.duration_ms, Ordering::Release);
        }

        ctx.ring.flush_and_wait();
        ctl.start_frame.store(ctx.ring.pushed.load(Ordering::Acquire), Ordering::Release);
        ctl.base_ms.store(target_ms, Ordering::Release);
        ctl.eof.store(false, Ordering::Release);
        ctl.ready.store(true, Ordering::Release);
        ctl.seeking.store(false, Ordering::Release);

        let mut resampler: Option<(u32, Resampler)> = None;
        let mut skip_frames: Option<u64> = None;

        loop {
            if let Some(flow) = check(ctx, &mut seen) {
                match flow {
                    Flow::Stop => return Ok(()),
                    Flow::Seek(ms) => {
                        target_ms = ms;
                        continue 'open;
                    }
                }
            }

            let packet = match opened.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(SymError::IoError(e)) if is_aborted(&e) => continue,
                Err(SymError::ResetRequired) => break,
                Err(e) => {
                    if ctl.abort.load(Ordering::Acquire) {
                        continue;
                    }
                    return Err(format!("ошибка чтения потока: {e}"));
                }
            };
            if packet.track_id() != opened.track_id {
                continue;
            }

            let decoded = match opened.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(e)) => {
                    tracing::debug!("пропущен битый пакет: {e}");
                    continue;
                }
                Err(SymError::IoError(e)) if is_aborted(&e) => continue,
                Err(e) => return Err(format!("ошибка декодирования: {e}")),
            };
            let spec = *decoded.spec();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let need = decoded.capacity() as u64;
            if sbuf.as_ref().map_or(true, |b| (b.capacity() as u64) < need * spec.channels.count() as u64) {
                sbuf = Some(SampleBuffer::<f32>::new(need, spec));
            }
            let sb = sbuf.as_mut().unwrap();
            sb.copy_interleaved_ref(decoded);

            let ch = spec.channels.count().max(1);
            stereo.clear();
            for f in sb.samples().chunks_exact(ch) {
                stereo.push(if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] });
            }

            let rate = spec.rate.max(1);
            let skip = skip_frames.get_or_insert_with(|| (opened.skip_ms.max(0.0) * rate as f64 / 1000.0) as u64);
            let mut slice: &[[f32; 2]] = &stereo;
            if *skip > 0 {
                let n = (*skip).min(slice.len() as u64) as usize;
                slice = &slice[n..];
                *skip -= n as u64;
            }
            if slice.is_empty() {
                continue;
            }

            if resampler.as_ref().map_or(true, |(r, _)| *r != rate) {
                resampler = Some((rate, Resampler::new(rate, ctx.ring.rate)));
            }
            out.clear();
            resampler.as_mut().unwrap().1.process(slice, &mut out);

            if let Some(flow) = push_all(ctx, &out, &mut seen) {
                match flow {
                    Flow::Stop => return Ok(()),
                    Flow::Seek(ms) => {
                        target_ms = ms;
                        continue 'open;
                    }
                }
            }
        }

        // конец потока: ждём, пока доиграет буфер, либо новой перемотки
        ctl.end_frame.store(ctx.ring.pushed.load(Ordering::Acquire), Ordering::Release);
        ctl.eof.store(true, Ordering::Release);
        loop {
            if let Some(flow) = check(ctx, &mut seen) {
                match flow {
                    Flow::Stop => return Ok(()),
                    Flow::Seek(ms) => {
                        target_ms = ms;
                        ctl.eof.store(false, Ordering::Release);
                        continue 'open;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

fn push_all(ctx: &DecodeCtx, mut rest: &[f32], seen: &mut u64) -> Option<Flow> {
    while !rest.is_empty() {
        if let Some(flow) = check(ctx, seen) {
            return Some(flow);
        }
        let pushed = {
            let mut prod = ctx.ring.producer.lock();
            let free = prod.slots() & !1;
            if free == 0 {
                0
            } else {
                let n = free.min(rest.len());
                let (done, _) = prod.push_partial_slice(&rest[..n]);
                done.len()
            }
        };
        if pushed == 0 {
            // буфер полон (идёт воспроизведение или пауза) — не крутим CPU
            let nap = if ctx.ring.paused.load(Ordering::Relaxed) { 40 } else { 8 };
            std::thread::sleep(Duration::from_millis(nap));
            continue;
        }
        ctx.ring.pushed.fetch_add((pushed / 2) as u64, Ordering::AcqRel);
        rest = &rest[pushed..];
    }
    None
}

fn probe(source: Box<dyn MediaSource>, ext: Option<&str>) -> Result<(Box<dyn FormatReader>, Box<dyn Decoder>, u32, Option<u64>), String> {
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions { buffer_len: 64 * 1024 });
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let fmt_opts = FormatOptions { enable_gapless: true, ..Default::default() };
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
        .map_err(|e| format!("формат не распознан: {e}"))?;
    let format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("в файле нет аудиодорожки")?;
    let track_id = track.id;
    let duration_ms = match (track.codec_params.n_frames, track.codec_params.time_base) {
        (Some(n), Some(tb)) => {
            let t = tb.calc_time(n);
            Some(t.seconds * 1000 + (t.frac * 1000.0) as u64)
        }
        (Some(n), None) => track.codec_params.sample_rate.map(|r| n * 1000 / r.max(1) as u64),
        _ => None,
    };
    let decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("кодек не поддерживается: {e}"))?;
    Ok((format, decoder, track_id, duration_ms))
}

fn ext_of(path: &std::path::Path) -> Option<String> {
    path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase())
}

fn open_at(ctx: &DecodeCtx, target_ms: u64) -> Result<Opened, String> {
    let ctl = &ctx.ctl;
    match &ctx.source {
        SourceHandle::File(path) => {
            let file = std::fs::File::open(path).map_err(|e| format!("не удалось открыть файл: {e}"))?;
            let (format, decoder, track_id, dur) = probe(Box::new(file), ext_of(path).as_deref())?;
            seekable_open(format, decoder, track_id, dur.unwrap_or(ctx.duration_hint_ms), target_ms)
        }
        SourceHandle::Prog(buf) => {
            buf.wait_bytes(16 * 1024, &ctl.abort)?;
            let ext = super::source::detect(&buf.head(64 * 1024)).ext();
            let reader = ProgReader::new(buf.clone(), ctl.abort.clone(), ctl.waiting.clone());
            let seekable = reader.is_seekable();
            let (format, decoder, track_id, dur) = probe(Box::new(reader), ext)?;
            let dur = dur.unwrap_or(ctx.duration_hint_ms);
            if seekable {
                seekable_open(format, decoder, track_id, dur, target_ms)
            } else {
                Ok(Opened { format, decoder, track_id, skip_ms: target_ms as f64, duration_ms: dur })
            }
        }
        SourceHandle::Hls(buf) => {
            let total_ms = buf.wait_playlist(&ctl.abort)?;
            let (seg, seg_start) = buf.segment_at(target_ms);
            buf.set_want(seg);
            let (_, container) = buf.wait_ready(&ctl.abort)?;
            if container == Container::Ogg {
                return Err("Opus-потоки не поддерживаются — выберите AAC или MP3 в настройках качества".into());
            }
            let reader = HlsReader::new(buf.clone(), seg, ctl.abort.clone(), ctl.waiting.clone());
            let (format, decoder, track_id, _) = probe(Box::new(reader), container.ext())?;
            Ok(Opened {
                format,
                decoder,
                track_id,
                skip_ms: target_ms.saturating_sub(seg_start) as f64,
                duration_ms: if total_ms > 0 { total_ms } else { ctx.duration_hint_ms },
            })
        }
    }
}

fn seekable_open(
    mut format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    duration_ms: u64,
    target_ms: u64,
) -> Result<Opened, String> {
    let mut skip_ms = 0.0;
    if target_ms > 0 {
        let time = Time::from(target_ms as f64 / 1000.0);
        match format.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(track_id) }) {
            Ok(seeked) => {
                if let Some(tb) = format.tracks().iter().find(|t| t.id == track_id).and_then(|t| t.codec_params.time_base) {
                    let req = tb.calc_time(seeked.required_ts);
                    let act = tb.calc_time(seeked.actual_ts);
                    let to_ms = |t: Time| t.seconds as f64 * 1000.0 + t.frac * 1000.0;
                    skip_ms = (to_ms(req) - to_ms(act)).max(0.0);
                }
            }
            Err(e) => {
                // формат не умеет перематываться — докручиваем декодированием
                tracing::debug!("seek не удался ({e}), пропускаем декодированием");
                skip_ms = target_ms as f64;
            }
        }
    }
    Ok(Opened { format, decoder, track_id, skip_ms, duration_ms })
}
