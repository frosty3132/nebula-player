//! Вывод на звуковое устройство через cpal (WASAPI на Windows).
//!
//! Колбэк реального времени только читает lock-free кольцевые буферы (rtrb),
//! прогоняет кадры через [`DspChain`] и пишет в буфер устройства.
//! Никаких аллокаций, системных вызовов и ожиданий внутри колбэка.
//!
//! Буферов два («дорожки»): обычно звучит одна, а при плавном переходе между
//! треками колбэк смешивает уходящую и следующую по равномощной кривой
//! (cos/sin), после чего следующая становится основной — см. [`Crossfade`].

use super::dsp::{DspChain, DspControl};
use super::spectrum::{Analyzer, Spectrum};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use parking_lot::Mutex;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Общее состояние кольцевого буфера (дорожки) между декодером и колбэком.
pub struct RingCtl {
    pub producer: Mutex<rtrb::Producer<f32>>,
    /// Кадров записано декодером (монотонно).
    pub pushed: AtomicU64,
    /// Кадров выведено или сброшено колбэком (монотонно).
    pub played: AtomicU64,
    flush_req: AtomicU64,
    flush_ack: AtomicU64,
    /// Общая пауза — одна на обе дорожки.
    pub paused: Arc<AtomicBool>,
    pub rate: u32,
}

/// Управление плавным переходом между дорожками (запросы — из потока плеера,
/// исполнение — в колбэке).
#[derive(Default)]
pub struct Crossfade {
    /// Дорожка, которая звучит сейчас (0 или 1).
    pub active: AtomicUsize,
    frames: AtomicU64,
    req: AtomicU64,
    done: AtomicU64,
    cut_to: AtomicUsize,
    cut_req: AtomicU64,
    cut_ack: AtomicU64,
}

impl Crossfade {
    /// Начать переход на другую дорожку длиной `frames` кадров.
    pub fn start(&self, frames: u64) {
        self.frames.store(frames.max(1), Ordering::Release);
        self.req.fetch_add(1, Ordering::AcqRel);
    }

    /// Идёт ли переход (запрошен, но колбэк его ещё не завершил).
    pub fn in_progress(&self) -> bool {
        self.done.load(Ordering::Acquire) < self.req.load(Ordering::Acquire)
    }

    /// Немедленно закончить переход, оставив звучать дорожку `lane`,
    /// и дождаться, пока колбэк это сделает.
    pub fn cut_and_wait(&self, lane: usize) {
        self.cut_to.store(lane, Ordering::Release);
        let req = self.cut_req.fetch_add(1, Ordering::AcqRel) + 1;
        let t0 = Instant::now();
        while self.cut_ack.load(Ordering::Acquire) < req {
            if t0.elapsed() > Duration::from_millis(300) {
                // колбэк не крутится (устройство пропало) — выставляем сами
                self.active.store(lane, Ordering::Release);
                self.done.store(self.req.load(Ordering::Acquire), Ordering::Release);
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl RingCtl {
    /// Попросить колбэк выбросить всё, что лежит в буфере, и дождаться подтверждения.
    pub fn flush_and_wait(&self) {
        let req = self.flush_req.fetch_add(1, Ordering::AcqRel) + 1;
        let t0 = Instant::now();
        while self.flush_ack.load(Ordering::Acquire) < req {
            if t0.elapsed() > Duration::from_millis(300) {
                // устройство не крутит колбэк (например, отключено) — не висим
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn buffered_frames(&self) -> u64 {
        self.pushed.load(Ordering::Acquire).saturating_sub(self.played.load(Ordering::Acquire))
    }
}

pub struct Output {
    _stream: cpal::Stream,
    /// Две дорожки: основная и следующая (для плавного перехода).
    pub lanes: [Arc<RingCtl>; 2],
    pub xfade: Arc<Crossfade>,
    pub paused: Arc<AtomicBool>,
    pub rate: u32,
    pub device_name: String,
    pub device_id: Option<String>,
    pub failed: Arc<AtomicBool>,
}

impl Output {
    pub fn lane(&self, i: usize) -> &Arc<RingCtl> {
        &self.lanes[i & 1]
    }

    /// Дорожка, которая звучит сейчас.
    pub fn active_lane(&self) -> usize {
        self.xfade.active.load(Ordering::Acquire) & 1
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OutputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn list_devices() -> Vec<OutputDevice> {
    let host = cpal::default_host();
    let default_id = host.default_output_device().and_then(|d| d.id().ok()).map(|i| i.to_string());
    let mut out = Vec::new();
    if let Ok(devs) = host.output_devices() {
        for d in devs {
            let Ok(id) = d.id() else { continue };
            let id = id.to_string();
            out.push(OutputDevice { is_default: Some(&id) == default_id.as_ref(), name: d.to_string(), id });
        }
    }
    out
}

pub fn default_device_id() -> Option<String> {
    cpal::default_host().default_output_device().and_then(|d| d.id().ok()).map(|i| i.to_string())
}

pub fn open(pref: Option<&str>, dsp: Arc<DspControl>, level: Arc<AtomicU32>, spectrum: Arc<Spectrum>) -> Result<Output, String> {
    let host = cpal::default_host();
    let device = pref
        .and_then(|id| cpal::DeviceId::from_str(id).ok())
        .and_then(|id| host.device_by_id(&id))
        .or_else(|| host.default_output_device())
        .ok_or("не найдено ни одного устройства вывода звука")?;
    let device_name = device.to_string();
    let device_id = device.id().ok().map(|i| i.to_string());
    let supported = device.default_output_config().map_err(|e| format!("устройство «{device_name}»: {e}"))?;
    let format = supported.sample_format();
    let config = supported.config();
    let rate = config.sample_rate;

    // 0.5 с стерео на дорожку: достаточно для сглаживания рывков планировщика,
    // и при этом перемотка/пауза срабатывают мгновенно
    let paused = Arc::new(AtomicBool::new(true));
    let mut consumers = Vec::with_capacity(2);
    let mut rings = Vec::with_capacity(2);
    for _ in 0..2 {
        let (producer, consumer) = rtrb::RingBuffer::<f32>::new(((rate as usize) / 2) * 2);
        consumers.push(consumer);
        rings.push(Arc::new(RingCtl {
            producer: Mutex::new(producer),
            pushed: AtomicU64::new(0),
            played: AtomicU64::new(0),
            flush_req: AtomicU64::new(0),
            flush_ack: AtomicU64::new(0),
            paused: paused.clone(),
            rate,
        }));
    }
    let lanes: [Arc<RingCtl>; 2] = [rings[0].clone(), rings[1].clone()];
    let cons: [rtrb::Consumer<f32>; 2] = {
        let c1 = consumers.pop().expect("дорожка 1");
        let c0 = consumers.pop().expect("дорожка 0");
        [c0, c1]
    };
    let xfade = Arc::new(Crossfade::default());
    let failed = Arc::new(AtomicBool::new(false));

    use cpal::SampleFormat as F;
    let args = (lanes.clone(), xfade.clone(), dsp, level, failed.clone(), spectrum);
    let stream = match format {
        F::F32 => build::<f32>(&device, config, cons, args),
        F::I16 => build::<i16>(&device, config, cons, args),
        F::U16 => build::<u16>(&device, config, cons, args),
        F::I32 => build::<i32>(&device, config, cons, args),
        F::F64 => build::<f64>(&device, config, cons, args),
        other => return Err(format!("формат сэмплов {other:?} не поддерживается")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    tracing::info!("аудиовыход: {device_name}, {rate} Гц, {} кан., {format:?}", config.channels);
    Ok(Output { _stream: stream, lanes, xfade, paused, rate, device_name, device_id, failed })
}

type BuildArgs = ([Arc<RingCtl>; 2], Arc<Crossfade>, Arc<DspControl>, Arc<AtomicU32>, Arc<AtomicBool>, Arc<Spectrum>);

/// Прочитать до `frames` кадров дорожки в `buf` (без аллокаций при достаточной ёмкости).
fn pull(consumer: &mut rtrb::Consumer<f32>, frames: usize, buf: &mut Vec<f32>) -> usize {
    buf.clear();
    let n = (consumer.slots() / 2).min(frames);
    if n == 0 {
        return 0;
    }
    if let Ok(chunk) = consumer.read_chunk(n * 2) {
        let (a, b) = chunk.as_slices();
        buf.extend_from_slice(a);
        buf.extend_from_slice(b);
        chunk.commit_all();
        n
    } else {
        0
    }
}

fn build<T>(device: &cpal::Device, config: cpal::StreamConfig, mut cons: [rtrb::Consumer<f32>; 2], args: BuildArgs) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let (lanes, xf, dsp, level, failed, spectrum) = args;
    let mut analyzer = Analyzer::new(config.sample_rate);
    let channels = config.channels.max(1) as usize;
    let mut chain = DspChain::new(config.sample_rate as f32);
    let mut acks = [0u64; 2];
    let mut seen_req = 0u64;
    let mut seen_cut = 0u64;
    // (кадров пройдено, всего) — пока идёт переход
    let mut fading: Option<(u64, u64)> = None;
    let mut buf_a: Vec<f32> = Vec::with_capacity(32_768);
    let mut buf_b: Vec<f32> = Vec::with_capacity(32_768);

    let data_cb = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        chain.refresh(&dsp);
        let mut active = xf.active.load(Ordering::Acquire) & 1;

        for i in 0..2 {
            let req = lanes[i].flush_req.load(Ordering::Acquire);
            if req != acks[i] {
                let n = cons[i].slots();
                if let Ok(chunk) = cons[i].read_chunk(n) {
                    chunk.commit_all();
                }
                lanes[i].played.fetch_add((n / 2) as u64, Ordering::AcqRel);
                acks[i] = req;
                lanes[i].flush_ack.store(req, Ordering::Release);
                if i == active && fading.is_none() {
                    chain.restart_fade();
                }
            }
        }

        // новый переход / досрочное завершение
        let req = xf.req.load(Ordering::Acquire);
        if req != seen_req {
            seen_req = req;
            fading = Some((0, xf.frames.load(Ordering::Acquire).max(1)));
        }
        let cut = xf.cut_req.load(Ordering::Acquire);
        if cut != seen_cut {
            seen_cut = cut;
            active = xf.cut_to.load(Ordering::Acquire) & 1;
            xf.active.store(active, Ordering::Release);
            fading = None;
            xf.done.store(seen_req, Ordering::Release);
            xf.cut_ack.store(cut, Ordering::Release);
        }

        let paused = lanes[0].paused.load(Ordering::Relaxed);
        let target = if paused { 0.0 } else { 1.0 };
        let frames = data.len() / channels;
        if buf_a.capacity() < frames * 2 {
            buf_a.reserve(frames * 2);
            buf_b.reserve(frames * 2);
        }

        // во время полной паузы буферы не трогаем — позиция стоит на месте
        let hold = paused && chain.is_silent();
        let other = active ^ 1;
        let na = if hold { 0 } else { pull(&mut cons[active], frames, &mut buf_a) };
        let nb = if hold || fading.is_none() { 0 } else { pull(&mut cons[other], frames, &mut buf_b) };
        // после завершения перехода посреди блока звучит уже вторая дорожка
        let mut switched = false;
        // анализатор спектра — только пока визуализатор на экране и есть звук
        let viz = !hold && spectrum.enabled.load(Ordering::Relaxed);

        for (i, frame) in data.chunks_mut(channels).enumerate() {
            let a = if i < na { (buf_a[2 * i], buf_a[2 * i + 1]) } else { (0.0, 0.0) };
            let b = if i < nb { (buf_b[2 * i], buf_b[2 * i + 1]) } else { (0.0, 0.0) };
            let (l, r) = if switched {
                b
            } else if let Some((pos, total)) = fading.as_mut() {
                let t = (*pos as f32 / *total as f32).min(1.0);
                let (ga, gb) = ((t * std::f32::consts::FRAC_PI_2).cos(), (t * std::f32::consts::FRAC_PI_2).sin());
                // переход идёт только пока есть звук — пауза и буферизация его не съедают
                if i < na || i < nb {
                    *pos += 1;
                }
                let mixed = (a.0 * ga + b.0 * gb, a.1 * ga + b.1 * gb);
                if *pos >= *total {
                    switched = true;
                }
                mixed
            } else {
                a
            };
            if viz {
                analyzer.push(0.5 * (l + r));
            }
            let (l, r) = chain.process(l, r, target);
            write_frame(frame, l, r);
        }

        if na > 0 {
            lanes[active].played.fetch_add(na as u64, Ordering::AcqRel);
        }
        if nb > 0 {
            lanes[other].played.fetch_add(nb as u64, Ordering::AcqRel);
        }
        if switched {
            fading = None;
            xf.active.store(other, Ordering::Release);
            xf.done.store(seen_req, Ordering::Release);
        }
        analyzer.publish(&spectrum, viz);
        chain.end_block();
        if let Some(lv) = chain.take_level() {
            level.store(lv.to_bits(), Ordering::Relaxed);
        }
    };

    let err_cb = move |err: cpal::Error| {
        tracing::warn!("ошибка аудиопотока: {err}");
        failed.store(true, Ordering::Relaxed);
    };

    device.build_output_stream::<T, _, _>(config, data_cb, err_cb, None).map_err(|e| e.to_string())
}

#[inline(always)]
fn write_frame<T: SizedSample + FromSample<f32>>(frame: &mut [T], l: f32, r: f32) {
    match frame.len() {
        1 => frame[0] = T::from_sample(0.5 * (l + r)),
        _ => {
            frame[0] = T::from_sample(l);
            frame[1] = T::from_sample(r);
            for s in frame.iter_mut().skip(2) {
                *s = T::from_sample(0.0f32);
            }
        }
    }
}
