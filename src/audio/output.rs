//! Вывод на звуковое устройство через cpal (WASAPI на Windows).
//!
//! Колбэк реального времени только читает lock-free кольцевой буфер (rtrb),
//! прогоняет кадры через [`DspChain`] и пишет в буфер устройства.
//! Никаких аллокаций, системных вызовов и ожиданий внутри колбэка.

use super::dsp::{DspChain, DspControl};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use parking_lot::Mutex;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Общее состояние кольцевого буфера между декодером и колбэком.
pub struct RingCtl {
    pub producer: Mutex<rtrb::Producer<f32>>,
    /// Кадров записано декодером (монотонно).
    pub pushed: AtomicU64,
    /// Кадров выведено или сброшено колбэком (монотонно).
    pub played: AtomicU64,
    flush_req: AtomicU64,
    flush_ack: AtomicU64,
    pub paused: AtomicBool,
    pub rate: u32,
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
    pub ring: Arc<RingCtl>,
    pub rate: u32,
    pub device_name: String,
    pub device_id: Option<String>,
    pub failed: Arc<AtomicBool>,
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

pub fn open(pref: Option<&str>, dsp: Arc<DspControl>, level: Arc<AtomicU32>) -> Result<Output, String> {
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

    // 0.5 с стерео: достаточно для сглаживания рывков планировщика,
    // и при этом перемотка/пауза срабатывают мгновенно
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(((rate as usize) / 2) * 2);
    let ring = Arc::new(RingCtl {
        producer: Mutex::new(producer),
        pushed: AtomicU64::new(0),
        played: AtomicU64::new(0),
        flush_req: AtomicU64::new(0),
        flush_ack: AtomicU64::new(0),
        paused: AtomicBool::new(true),
        rate,
    });
    let failed = Arc::new(AtomicBool::new(false));

    use cpal::SampleFormat as F;
    let stream = match format {
        F::F32 => build::<f32>(&device, config, consumer, ring.clone(), dsp, level, failed.clone()),
        F::I16 => build::<i16>(&device, config, consumer, ring.clone(), dsp, level, failed.clone()),
        F::U16 => build::<u16>(&device, config, consumer, ring.clone(), dsp, level, failed.clone()),
        F::I32 => build::<i32>(&device, config, consumer, ring.clone(), dsp, level, failed.clone()),
        F::F64 => build::<f64>(&device, config, consumer, ring.clone(), dsp, level, failed.clone()),
        other => return Err(format!("формат сэмплов {other:?} не поддерживается")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    tracing::info!("аудиовыход: {device_name}, {rate} Гц, {} кан., {format:?}", config.channels);
    Ok(Output { _stream: stream, ring, rate, device_name, device_id, failed })
}

fn build<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut consumer: rtrb::Consumer<f32>,
    ring: Arc<RingCtl>,
    dsp: Arc<DspControl>,
    level: Arc<AtomicU32>,
    failed: Arc<AtomicBool>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let mut chain = DspChain::new(config.sample_rate as f32);
    let mut ack = 0u64;

    let data_cb = move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        chain.refresh(&dsp);

        let req = ring.flush_req.load(Ordering::Acquire);
        if req != ack {
            let n = consumer.slots();
            if let Ok(chunk) = consumer.read_chunk(n) {
                chunk.commit_all();
            }
            ring.played.fetch_add((n / 2) as u64, Ordering::AcqRel);
            ack = req;
            ring.flush_ack.store(req, Ordering::Release);
            chain.restart_fade();
        }

        let paused = ring.paused.load(Ordering::Relaxed);
        let target = if paused { 0.0 } else { 1.0 };
        let frames = data.len() / channels;
        let mut played = 0u64;

        // во время полной паузы буфер не трогаем — позиция стоит на месте
        let readable = if paused && chain.is_silent() { 0 } else { (consumer.slots() / 2).min(frames) };
        let mut frames_iter = data.chunks_mut(channels);
        if readable > 0 {
            if let Ok(chunk) = consumer.read_chunk(readable * 2) {
                let (a, b) = chunk.as_slices();
                let mut it = a.iter().chain(b.iter());
                while let (Some(&l), Some(&r)) = (it.next(), it.next()) {
                    let (l, r) = chain.process(l, r, target);
                    if let Some(frame) = frames_iter.next() {
                        write_frame(frame, l, r);
                    }
                    played += 1;
                }
                chunk.commit_all();
            }
        }
        // недобор (буферизация/пауза) — тишина, огибающая продолжает идти
        for frame in frames_iter {
            let (l, r) = chain.process(0.0, 0.0, target);
            write_frame(frame, l, r);
        }

        if played > 0 {
            ring.played.fetch_add(played, Ordering::AcqRel);
        }
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
