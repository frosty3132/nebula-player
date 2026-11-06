//! Контроллер плеера: отдельный поток, который владеет устройством вывода,
//! текущей сессией декодирования и предзагрузкой следующего трека.
//! UI общается с ним командами через канал и читает готовый [`Snapshot`].

use super::decode::{self, DecodeCtx, SessionCtl, SourceHandle};
use super::dsp::{DspControl, DspParams};
use super::output::{self, Output};
use super::source::{HlsBuffer, ProgBuffer};
use super::spectrum::{Spectrum, BANDS};
pub use super::output::OutputDevice;
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

/// Что именно играть.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaSpec {
    File(PathBuf),
    /// Прогрессивный HTTP (например, 30-секундное превью MP3).
    Http { url: String, auth: Option<String> },
    /// HLS-плейлист (AAC/MP3).
    Hls { url: String, auth: Option<String> },
}

impl MediaSpec {
    fn key(&self) -> String {
        match self {
            MediaSpec::File(p) => format!("file:{}", p.display()),
            MediaSpec::Http { url, .. } => format!("http:{url}"),
            MediaSpec::Hls { url, .. } => format!("hls:{url}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Idle,
    Loading,
    Buffering,
    Playing,
    Paused,
    Ended,
    Error,
}

impl PlaybackState {
    pub fn is_active(self) -> bool {
        matches!(self, PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Loading)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub load_id: u64,
    pub state: PlaybackState,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub buffered: f32,
    pub device: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    Started { load_id: u64 },
    Ended { load_id: u64 },
    Error { load_id: u64, message: String },
    DeviceChanged(String),
}

enum Cmd {
    Load { id: u64, spec: MediaSpec, start_ms: u64, autoplay: bool, duration_hint_ms: u64 },
    Play,
    Pause,
    Toggle,
    Seek(u64),
    Preload(MediaSpec),
    SetDevice(Option<String>),
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    dsp: Arc<DspControl>,
    level: Arc<AtomicU32>,
    spectrum: Arc<Spectrum>,
    read_ahead_ms: AtomicU64,
    /// Длительность плавного перехода между треками (0 — без перехода).
    crossfade_ms: AtomicU64,
}

pub struct Player {
    tx: Sender<Cmd>,
    shared: Arc<Shared>,
    events: Mutex<Option<UnboundedReceiver<PlayerEvent>>>,
}

impl Player {
    pub fn new(http: reqwest::Client, rt: Handle, dsp: DspParams, device: Option<String>) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (ev_tx, ev_rx) = unbounded_channel();
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            dsp: Arc::new(DspControl::new(dsp)),
            level: Arc::new(AtomicU32::new(0)),
            spectrum: Arc::new(Spectrum::default()),
            read_ahead_ms: AtomicU64::new(240_000),
            crossfade_ms: AtomicU64::new(0),
        });
        let sh = shared.clone();
        std::thread::Builder::new()
            .name("nebula-player".into())
            .spawn(move || Controller::new(rx, sh, ev_tx, http, rt, device).run())
            .expect("не удалось запустить поток плеера");
        Self { tx, shared, events: Mutex::new(Some(ev_rx)) }
    }

    pub fn load(&self, id: u64, spec: MediaSpec, start_ms: u64, autoplay: bool, duration_hint_ms: u64) {
        let _ = self.tx.send(Cmd::Load { id, spec, start_ms, autoplay, duration_hint_ms });
    }
    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn toggle(&self) {
        let _ = self.tx.send(Cmd::Toggle);
    }
    pub fn seek(&self, ms: u64) {
        let _ = self.tx.send(Cmd::Seek(ms));
    }
    pub fn preload(&self, spec: MediaSpec) {
        let _ = self.tx.send(Cmd::Preload(spec));
    }
    pub fn set_device(&self, id: Option<String>) {
        let _ = self.tx.send(Cmd::SetDevice(id));
    }
    pub fn set_dsp(&self, p: DspParams) {
        self.shared.dsp.set(p);
    }
    pub fn set_read_ahead(&self, ms: u64) {
        self.shared.read_ahead_ms.store(ms.max(15_000), Ordering::Relaxed);
    }
    /// Плавный переход между треками, как «Кроссфейд» в Spotify (0 — выключен).
    pub fn set_crossfade(&self, ms: u64) {
        self.shared.crossfade_ms.store(ms.min(12_000), Ordering::Relaxed);
    }
    pub fn snapshot(&self) -> Snapshot {
        self.shared.snapshot.lock().clone()
    }
    /// Включить анализатор спектра (пока визуализатор виден на экране).
    pub fn set_visualizer(&self, on: bool) {
        self.shared.spectrum.enabled.store(on, Ordering::Relaxed);
    }
    /// Полосы спектра 0..1, от низких частот к высоким.
    pub fn spectrum(&self) -> [f32; BANDS] {
        self.shared.spectrum.read()
    }
    /// Уровень низких частот на выходе, 0..~1 (для визуальной пульсации).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }
    pub fn take_events(&self) -> Option<UnboundedReceiver<PlayerEvent>> {
        self.events.lock().take()
    }
    pub fn devices() -> Vec<OutputDevice> {
        output::list_devices()
    }
}

struct Session {
    /// 0 — трек плавного перехода, который интерфейс ещё не «принял» загрузкой.
    id: u64,
    spec: MediaSpec,
    source: SourceHandle,
    ctl: Arc<SessionCtl>,
    thread: Option<JoinHandle<()>>,
    duration_hint_ms: u64,
    /// Дорожка вывода, в которую пишет декодер.
    lane: usize,
    started: bool,
    ended: bool,
    errored: bool,
}

impl Session {
    fn shutdown(&mut self) {
        self.ctl.request_stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// Остановить декодер и отпустить загрузку источника.
    fn close(mut self) {
        self.shutdown();
        self.source.cancel();
    }
}

struct Controller {
    rx: Receiver<Cmd>,
    shared: Arc<Shared>,
    events: UnboundedSender<PlayerEvent>,
    http: reqwest::Client,
    rt: Handle,
    out: Option<Output>,
    device_pref: Option<String>,
    sess: Option<Session>,
    /// Следующий трек, заранее запущенный на второй дорожке для плавного перехода.
    next: Option<Session>,
    /// Уходящий трек: доигрывает, пока звучит переход.
    fading: Option<Session>,
    /// С какого момента переход звучит, а интерфейс ещё не загрузил следующий трек.
    unadopted_since: Option<Instant>,
    preload: Option<(MediaSpec, SourceHandle)>,
    last_device_check: Instant,
    last_default: Option<String>,
    last_open_attempt: Instant,
}

impl Controller {
    fn new(rx: Receiver<Cmd>, shared: Arc<Shared>, events: UnboundedSender<PlayerEvent>, http: reqwest::Client, rt: Handle, device_pref: Option<String>) -> Self {
        let mut c = Self {
            rx,
            shared,
            events,
            http,
            rt,
            out: None,
            device_pref,
            sess: None,
            next: None,
            fading: None,
            unadopted_since: None,
            preload: None,
            last_device_check: Instant::now(),
            last_default: output::default_device_id(),
            last_open_attempt: Instant::now() - Duration::from_secs(10),
        };
        c.ensure_output();
        c
    }

    fn ensure_output(&mut self) -> bool {
        if self.out.is_some() {
            return true;
        }
        if self.last_open_attempt.elapsed() < Duration::from_secs(2) {
            return false;
        }
        self.last_open_attempt = Instant::now();
        match output::open(self.device_pref.as_deref(), self.shared.dsp.clone(), self.shared.level.clone(), self.shared.spectrum.clone()) {
            Ok(o) => {
                let _ = self.events.send(PlayerEvent::DeviceChanged(o.device_name.clone()));
                self.shared.snapshot.lock().device = o.device_name.clone();
                self.out = Some(o);
                true
            }
            Err(e) => {
                tracing::error!("не удалось открыть аудиовыход: {e}");
                false
            }
        }
    }

    fn run(mut self) {
        loop {
            match self.rx.recv_timeout(Duration::from_millis(40)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(cmd) = self.rx.try_recv() {
                self.handle(cmd);
            }
            self.tick();
        }
        for s in [self.sess.take(), self.next.take(), self.fading.take()].into_iter().flatten() {
            s.close();
        }
    }

    fn make_source(&mut self, spec: &MediaSpec) -> SourceHandle {
        let key = spec.key();
        if let Some((p, src)) = self.preload.take() {
            if p.key() == key {
                return src;
            }
            src.cancel();
        }
        let ahead = self.shared.read_ahead_ms.load(Ordering::Relaxed);
        match spec {
            MediaSpec::File(p) => SourceHandle::File(p.clone()),
            MediaSpec::Http { url, auth } => SourceHandle::Prog(ProgBuffer::start(self.http.clone(), &self.rt, url.clone(), auth.clone())),
            MediaSpec::Hls { url, auth } => SourceHandle::Hls(HlsBuffer::start(self.http.clone(), &self.rt, url.clone(), auth.clone(), ahead)),
        }
    }

    fn spawn_decoder(&mut self, source: SourceHandle, start_ms: u64, hint: u64, lane: usize) -> Option<(Arc<SessionCtl>, JoinHandle<()>)> {
        let out = self.out.as_ref()?;
        let ctl = Arc::new(SessionCtl::default());
        ctl.duration_ms.store(hint, Ordering::Relaxed);
        let thread = decode::spawn(DecodeCtx { source, ctl: ctl.clone(), ring: out.lane(lane).clone(), start_ms, duration_hint_ms: hint });
        Some((ctl, thread))
    }

    /// Закончить переход сразу: звучать остаётся дорожка текущего трека.
    fn finish_crossfade_now(&mut self) {
        if let Some(f) = self.fading.take() {
            if let (Some(out), Some(s)) = (&self.out, &self.sess) {
                out.xfade.cut_and_wait(s.lane);
            }
            f.close();
        }
    }

    /// Убрать всё, что связано с плавным переходом: заготовленный следующий
    /// трек и уходящий. Звучать остаётся дорожка текущего трека.
    fn drop_crossfade(&mut self) {
        if let Some(n) = self.next.take() {
            n.close();
        }
        self.finish_crossfade_now();
        self.unadopted_since = None;
    }

    /// Интерфейс не принял переход (например, «повтор трека») — вернуться к уходящему.
    fn revert_crossfade(&mut self) {
        let Some(mut old) = self.fading.take() else { return };
        if let Some(out) = &self.out {
            out.xfade.cut_and_wait(old.lane);
        }
        if let Some(n) = self.sess.take() {
            n.close();
        }
        old.ended = false;
        self.sess = Some(old);
        self.unadopted_since = None;
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Load { id, spec, start_ms, autoplay, duration_hint_ms } => {
                // интерфейс перешёл на трек, который уже звучит в плавном переходе
                if let Some(s) = &mut self.sess {
                    if s.id == 0 && start_ms == 0 && s.spec.key() == spec.key() {
                        s.id = id;
                        if duration_hint_ms > 0 && s.ctl.duration_ms.load(Ordering::Acquire) == 0 {
                            s.ctl.duration_ms.store(duration_hint_ms, Ordering::Release);
                        }
                        s.duration_hint_ms = duration_hint_ms;
                        self.unadopted_since = None;
                        if !autoplay {
                            self.set_paused(true);
                        }
                        return;
                    }
                }
                self.drop_crossfade();
                let mut reuse = None;
                if let Some(mut old) = self.sess.take() {
                    old.shutdown();
                    if old.spec.key() == spec.key() {
                        reuse = Some(old.source.clone());
                    } else {
                        old.source.cancel();
                    }
                }
                if !self.ensure_output() {
                    let _ = self.events.send(PlayerEvent::Error { load_id: id, message: "нет доступного устройства вывода звука".into() });
                    return;
                }
                let source = match reuse {
                    Some(s) => s,
                    None => self.make_source(&spec),
                };
                let Some(lane) = self.out.as_ref().map(|o| {
                    o.paused.store(!autoplay, Ordering::Release);
                    o.active_lane()
                }) else {
                    return;
                };
                let Some((ctl, thread)) = self.spawn_decoder(source.clone(), start_ms, duration_hint_ms, lane) else { return };
                self.sess = Some(Session { id, spec, source, ctl, thread: Some(thread), duration_hint_ms, lane, started: false, ended: false, errored: false });
            }
            Cmd::Play => self.set_paused(false),
            Cmd::Pause => self.set_paused(true),
            Cmd::Toggle => {
                let paused = self.out.as_ref().map(|o| o.paused.load(Ordering::Acquire)).unwrap_or(true);
                self.set_paused(!paused);
            }
            Cmd::Seek(ms) => {
                // перемотка во время перехода: «повтор трека» возвращает уходящий,
                // иначе переход заканчивается сразу и перематывается новый трек
                if self.sess.as_ref().is_some_and(|s| s.id == 0) {
                    self.revert_crossfade();
                } else {
                    self.finish_crossfade_now();
                }
                if let Some(s) = &mut self.sess {
                    s.ended = false;
                    s.ctl.request_seek(ms);
                }
            }
            Cmd::Preload(spec) => {
                let key = spec.key();
                let current = self.sess.as_ref().map(|s| s.spec.key());
                if current.as_deref() == Some(key.as_str())
                    || self.preload.as_ref().is_some_and(|(p, _)| p.key() == key)
                    || self.next.as_ref().is_some_and(|n| n.spec.key() == key)
                {
                    return;
                }
                // очередь поменялась — заготовленный переход больше не нужен
                if let Some(n) = self.next.take() {
                    n.close();
                }
                if let Some((_, old)) = self.preload.take() {
                    old.cancel();
                }
                let src = match &spec {
                    MediaSpec::File(p) => SourceHandle::File(p.clone()),
                    MediaSpec::Http { url, auth } => SourceHandle::Prog(ProgBuffer::start(self.http.clone(), &self.rt, url.clone(), auth.clone())),
                    // для предзагрузки достаточно первых ~40 секунд
                    MediaSpec::Hls { url, auth } => SourceHandle::Hls(HlsBuffer::start(self.http.clone(), &self.rt, url.clone(), auth.clone(), 40_000)),
                };
                self.preload = Some((spec, src));
            }
            Cmd::SetDevice(pref) => {
                self.device_pref = pref;
                self.reopen_output();
            }
        }
    }

    fn set_paused(&mut self, paused: bool) {
        let Some(out) = &self.out else { return };
        // «Play» после окончания трека — начать заново
        if !paused {
            if let Some(s) = &mut self.sess {
                if s.ended || s.ctl.eof.load(Ordering::Acquire) && out.lane(s.lane).buffered_frames() == 0 {
                    s.ended = false;
                    s.ctl.request_seek(0);
                }
            }
        }
        out.paused.store(paused, Ordering::Release);
    }

    /// Пересоздать вывод (смена/потеря устройства) и продолжить с той же позиции.
    fn reopen_output(&mut self) {
        let pos = self.position_ms();
        let paused = self.out.as_ref().map(|o| o.paused.load(Ordering::Acquire)).unwrap_or(true);
        // переход не переносим на новое устройство — остаётся текущий трек
        if self.sess.as_ref().is_some_and(|s| s.id == 0) {
            self.revert_crossfade();
        }
        if let Some(n) = self.next.take() {
            n.close();
        }
        if let Some(f) = self.fading.take() {
            f.close();
        }
        self.unadopted_since = None;
        if let Some(s) = &mut self.sess {
            s.ctl.request_stop();
            if let Some(t) = s.thread.take() {
                let _ = t.join();
            }
        }
        self.out = None;
        self.last_open_attempt = Instant::now() - Duration::from_secs(10);
        if !self.ensure_output() {
            return;
        }
        if let Some(out) = &self.out {
            out.paused.store(paused, Ordering::Release);
        }
        if let Some(s) = self.sess.take() {
            let lane = self.out.as_ref().map(|o| o.active_lane()).unwrap_or(0);
            if let Some((ctl, thread)) = self.spawn_decoder(s.source.clone(), pos, s.duration_hint_ms, lane) {
                self.sess = Some(Session { ctl, thread: Some(thread), started: s.started, lane, ..s });
            }
        }
    }

    fn position_ms(&self) -> u64 {
        let (Some(s), Some(out)) = (&self.sess, &self.out) else { return 0 };
        if s.ctl.seeking.load(Ordering::Acquire) || !s.ctl.ready.load(Ordering::Acquire) {
            return if s.ctl.seeking.load(Ordering::Acquire) { s.ctl.seek_ms.load(Ordering::Acquire) } else { s.ctl.base_ms.load(Ordering::Acquire) };
        }
        let played = out.lane(s.lane).played.load(Ordering::Acquire);
        let start = s.ctl.start_frame.load(Ordering::Acquire);
        let base = s.ctl.base_ms.load(Ordering::Acquire);
        let pos = base + played.saturating_sub(start) * 1000 / out.rate.max(1) as u64;
        let dur = s.ctl.duration_ms.load(Ordering::Acquire);
        if dur > 0 {
            pos.min(dur)
        } else {
            pos
        }
    }

    /// Плавный переход между треками: заранее запустить следующий трек на
    /// второй дорожке, а за `crossfade_ms` до конца — начать смешивание.
    fn crossfade_tick(&mut self) {
        let Some(out) = &self.out else { return };
        // переход доиграл — уходящий трек больше не нужен
        if self.fading.is_some() && !out.xfade.in_progress() && self.sess.as_ref().is_some_and(|s| s.id != 0) {
            if let Some(f) = self.fading.take() {
                f.close();
            }
        }
        // интерфейс так и не загрузил следующий трек — возвращаемся к прежнему
        if self.unadopted_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            self.revert_crossfade();
            if let (Some(s), Some(out)) = (&mut self.sess, &self.out) {
                s.ended = true;
                out.paused.store(true, Ordering::Release);
            }
            return;
        }
        let xf_ms = self.shared.crossfade_ms.load(Ordering::Relaxed);
        if xf_ms == 0 {
            if let Some(n) = self.next.take() {
                n.close();
            }
            return;
        }
        if self.fading.is_some() {
            return;
        }
        let pos = self.position_ms();
        let Some(out) = &self.out else { return };
        let paused = out.paused.load(Ordering::Acquire);
        let rate = out.rate as u64;
        let Some(s) = &self.sess else { return };
        if s.id == 0 || !s.started || s.ended || s.errored || s.ctl.seeking.load(Ordering::Acquire) {
            return;
        }
        let dur = s.ctl.duration_ms.load(Ordering::Acquire);
        // короткие треки и превью не смешиваем
        if dur < xf_ms * 2 + 5_000 {
            return;
        }
        let remaining = dur.saturating_sub(pos);
        let cur_key = s.spec.key();
        let cur_lane = s.lane;

        // заранее (за несколько секунд) запускаем декодер следующего трека
        if self.next.is_none() && remaining <= xf_ms + 6_000 && remaining > xf_ms / 2 {
            if let Some((spec, src)) = self.preload.take() {
                if spec.key() == cur_key {
                    self.preload = Some((spec, src));
                    return;
                }
                let lane = cur_lane ^ 1;
                if let Some((ctl, thread)) = self.spawn_decoder(src.clone(), 0, 0, lane) {
                    self.next = Some(Session { id: 0, spec, source: src, ctl, thread: Some(thread), duration_hint_ms: 0, lane, started: false, ended: false, errored: false });
                }
            }
        }

        let Some(n) = &self.next else { return };
        if n.ctl.error.lock().is_some() {
            // следующий трек не открылся — обычный переход по окончании
            if let Some(n) = self.next.take() {
                n.close();
            }
            return;
        }
        let Some(out) = &self.out else { return };
        let ready = n.ctl.ready.load(Ordering::Acquire) && out.lane(n.lane).buffered_frames() >= rate / 4;
        if !ready || paused || remaining > xf_ms {
            return;
        }
        out.xfade.start(remaining.min(xf_ms) * rate / 1000);
        let Some(mut old) = self.sess.take() else { return };
        old.ended = true;
        // для интерфейса уходящий трек закончился — он загрузит следующий
        let _ = self.events.send(PlayerEvent::Ended { load_id: old.id });
        self.fading = Some(old);
        self.sess = self.next.take();
        self.unadopted_since = Some(Instant::now());
    }

    fn tick(&mut self) {
        // потеря устройства или смена устройства по умолчанию в системе
        let failed = self.out.as_ref().map(|o| o.failed.load(Ordering::Relaxed)).unwrap_or(false);
        if failed {
            tracing::warn!("аудиоустройство потеряно — переподключаемся");
            self.reopen_output();
        } else if self.out.is_none() {
            if self.ensure_output() && self.sess.is_some() {
                self.reopen_output();
            }
        } else if self.device_pref.is_none() && self.last_device_check.elapsed() > Duration::from_secs(2) {
            self.last_device_check = Instant::now();
            let now = output::default_device_id();
            if now.is_some() && now != self.last_default {
                self.last_default = now;
                if self.out.as_ref().and_then(|o| o.device_id.clone()) != self.last_default {
                    tracing::info!("устройство по умолчанию сменилось — переключаемся");
                    self.reopen_output();
                }
            }
        }

        self.crossfade_tick();

        let position = self.position_ms();
        let device = self.out.as_ref().map(|o| o.device_name.clone()).unwrap_or_default();
        let mut snap = Snapshot { device, ..Default::default() };
        if let (Some(s), Some(out)) = (&mut self.sess, &self.out) {
            let ctl = &s.ctl;
            let lane = out.lane(s.lane);
            let paused = out.paused.load(Ordering::Acquire);
            let err = ctl.error.lock().clone();
            let ended = ctl.eof.load(Ordering::Acquire) && lane.played.load(Ordering::Acquire) >= ctl.end_frame.load(Ordering::Acquire);
            let state = if err.is_some() {
                PlaybackState::Error
            } else if !ctl.ready.load(Ordering::Acquire) {
                PlaybackState::Loading
            } else if ended {
                PlaybackState::Ended
            } else if paused {
                PlaybackState::Paused
            } else if (ctl.waiting.load(Ordering::Relaxed) || ctl.seeking.load(Ordering::Acquire)) && lane.buffered_frames() < (out.rate as u64 / 8) {
                PlaybackState::Buffering
            } else {
                PlaybackState::Playing
            };

            // события — только для треков, которые интерфейс загрузил сам
            if s.id != 0 {
                if let Some(e) = &err {
                    if !s.errored {
                        s.errored = true;
                        let _ = self.events.send(PlayerEvent::Error { load_id: s.id, message: e.clone() });
                    }
                }
                if state == PlaybackState::Playing && !s.started {
                    s.started = true;
                    let _ = self.events.send(PlayerEvent::Started { load_id: s.id });
                }
                if ended && !s.ended {
                    s.ended = true;
                    let _ = self.events.send(PlayerEvent::Ended { load_id: s.id });
                }
            }
            snap.load_id = s.id;
            snap.state = state;
            snap.position_ms = position;
            snap.duration_ms = ctl.duration_ms.load(Ordering::Acquire);
            snap.buffered = s.source.progress();
            snap.error = err;
        }
        *self.shared.snapshot.lock() = snap;
    }
}

/// Сквозные тесты движка на реальном устройстве (громкость 0 — без звука).
/// Запуск: `cargo test live_ -- --ignored --nocapture` (нужны звуковая карта и интернет).
#[cfg(test)]
mod live_tests {
    use super::*;

    fn wait(player: &Player, what: &str, secs: u64, ok: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let t0 = Instant::now();
        loop {
            let s = player.snapshot();
            assert!(s.state != PlaybackState::Error, "{what}: ошибка {:?}", s.error);
            if ok(&s) {
                return s;
            }
            assert!(t0.elapsed() < Duration::from_secs(secs), "{what}: таймаут, {s:?}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn run(spec: MediaSpec, seek_to: u64) {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
        let player = Player::new(reqwest::Client::new(), rt.handle().clone(), DspParams { gain: 0.0, ..Default::default() }, None);
        let t0 = Instant::now();
        player.load(1, spec, 0, true, 0);
        let s = wait(&player, "старт", 30, |s| s.state == PlaybackState::Playing && s.position_ms >= 1500);
        println!("играет через {:?}, длительность {} мс, буфер {:.0}%", t0.elapsed(), s.duration_ms, s.buffered * 100.0);
        let t1 = Instant::now();
        player.seek(seek_to);
        let s = wait(&player, "перемотка", 30, |s| s.state == PlaybackState::Playing && s.position_ms >= seek_to + 700);
        println!("после перемотки на {seek_to} мс: позиция {} мс за {:?}", s.position_ms, t1.elapsed());
        assert!(s.position_ms < seek_to + 5000, "позиция ушла слишком далеко: {}", s.position_ms);
        player.pause();
        std::thread::sleep(Duration::from_millis(600));
        let p1 = player.snapshot().position_ms;
        std::thread::sleep(Duration::from_millis(800));
        let p2 = player.snapshot().position_ms;
        assert_eq!(p1, p2, "на паузе позиция не должна двигаться");
        assert_eq!(player.snapshot().state, PlaybackState::Paused);
    }

    #[test]
    #[ignore]
    fn live_hls_fmp4_aac_byterange() {
        run(
            MediaSpec::Hls { url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/img_bipbop_adv_example_fmp4/a1/prog_index.m3u8".into(), auth: None },
            60_000,
        );
    }

    #[test]
    #[ignore]
    fn live_hls_packed_adts_aac() {
        run(
            MediaSpec::Hls { url: "https://devstreaming-cdn.apple.com/videos/streaming/examples/bipbop_4x3/gear0/prog_index.m3u8".into(), auth: None },
            60_000,
        );
    }

    fn silence_wav(name: &str, secs: u32) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        let rate = 44_100u32;
        let data_len = rate * 4 * secs;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        std::fs::write(&path, wav).unwrap();
        path
    }

    /// Плавный переход: первый трек «заканчивается» за `crossfade` до конца,
    /// второй уже звучит и принимается загрузкой без перезапуска.
    #[test]
    #[ignore]
    fn live_crossfade_between_files() {
        let a = silence_wav("nebula_xf_a.wav", 12);
        let b = silence_wav("nebula_xf_b.wav", 12);
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let player = Player::new(reqwest::Client::new(), rt.handle().clone(), DspParams { gain: 0.0, ..Default::default() }, None);
        player.set_crossfade(2_000);
        let mut events = player.take_events().unwrap();
        let t0 = Instant::now();
        player.load(1, MediaSpec::File(a.clone()), 0, true, 0);
        wait(&player, "старт", 10, |s| s.state == PlaybackState::Playing);
        player.preload(MediaSpec::File(b.clone()));

        // событие конца первого трека приходит в начале перехода, а не в самом конце
        let ended_at = loop {
            if let Ok(PlayerEvent::Ended { load_id: 1 }) = events.try_recv() {
                break t0.elapsed();
            }
            assert!(t0.elapsed() < Duration::from_secs(16), "нет события конца первого трека");
            std::thread::sleep(Duration::from_millis(20));
        };
        println!("переход начался через {ended_at:?}");
        assert!(ended_at > Duration::from_millis(8_500) && ended_at < Duration::from_millis(11_500), "{ended_at:?}");

        // интерфейс загружает следующий трек — он уже звучит, перезапуска нет
        std::thread::sleep(Duration::from_millis(150));
        player.load(2, MediaSpec::File(b.clone()), 0, true, 0);
        std::thread::sleep(Duration::from_millis(400));
        let s = player.snapshot();
        println!("после принятия: {s:?}");
        assert_eq!(s.load_id, 2);
        assert_eq!(s.state, PlaybackState::Playing);
        assert!(s.position_ms >= 300 && s.position_ms < 2_000, "второй трек должен продолжаться: {} мс", s.position_ms);

        // второй трек доигрывает до конца (12 с от начала перехода)
        let s = wait(&player, "конец второго", 16, |s| s.state == PlaybackState::Ended);
        println!("второй закончился на {} мс, всего {:?}", s.position_ms, t0.elapsed());
        assert!(t0.elapsed() > Duration::from_millis(20_500), "второй трек не должен обрываться: {:?}", t0.elapsed());
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    #[ignore]
    fn live_local_wav_to_end() {
        // 3 секунды тишины 44.1 кГц: проверяем ресемплинг до частоты устройства и событие конца
        let path = std::env::temp_dir().join("nebula_test_silence.wav");
        let (rate, secs) = (44_100u32, 3u32);
        let data_len = rate * 4 * secs;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        std::fs::write(&path, wav).unwrap();

        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
        let player = Player::new(reqwest::Client::new(), rt.handle().clone(), DspParams { gain: 0.0, ..Default::default() }, None);
        let mut events = player.take_events().unwrap();
        player.load(7, MediaSpec::File(path.clone()), 0, true, 0);
        let s = wait(&player, "файл", 10, |s| s.state == PlaybackState::Ended);
        assert!((2900..=3100).contains(&s.position_ms), "конец на {} мс", s.position_ms);
        assert_eq!(s.duration_ms, 3000);
        let mut got_end = false;
        while let Ok(ev) = events.try_recv() {
            got_end |= ev == PlayerEvent::Ended { load_id: 7 };
        }
        assert!(got_end, "должно прийти событие Ended");
        let _ = std::fs::remove_file(path);
    }
}
