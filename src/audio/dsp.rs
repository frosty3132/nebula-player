//! DSP-цепочка, работающая прямо в аудиоколбэке: эквалайзер (10 биквадов),
//! предусилитель, баланс/моно, сглаженная громкость до 400%, огибающая паузы
//! и пиковый лимитер с мягким клиппером. Никаких аллокаций и блокировок —
//! параметры забираются через `try_lock`, только когда UI их поменял.

use parking_lot::Mutex;
use std::f32::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering};

/// Центральные частоты полос эквалайзера, Гц.
pub const EQ_FREQS: [f32; 10] = [
    31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DspParams {
    /// Итоговый линейный коэффициент громкости (кривая уже применена). 2.0 = +6 дБ.
    pub gain: f32,
    pub eq_enabled: bool,
    pub eq_db: [f32; 10],
    pub preamp_db: f32,
    /// -1.0 — только левый канал, 1.0 — только правый.
    pub balance: f32,
    pub mono: bool,
    pub limiter: bool,
    /// Длительность плавного затухания/нарастания при паузе и перемотке.
    pub fade_ms: f32,
}

impl Default for DspParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            eq_enabled: false,
            eq_db: [0.0; 10],
            preamp_db: 0.0,
            balance: 0.0,
            mono: false,
            limiter: true,
            fade_ms: 120.0,
        }
    }
}

/// Общий контейнер параметров между UI и аудиопотоком.
pub struct DspControl {
    params: Mutex<DspParams>,
    version: AtomicU64,
}

impl DspControl {
    pub fn new(p: DspParams) -> Self {
        Self { params: Mutex::new(p), version: AtomicU64::new(1) }
    }

    pub fn set(&self, p: DspParams) {
        *self.params.lock() = p;
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Неблокирующая подгрузка новых параметров (для реального времени).
    fn try_load(&self, seen: &mut u64, out: &mut DspParams) -> bool {
        let v = self.version.load(Ordering::Acquire);
        if v == *seen {
            return false;
        }
        match self.params.try_lock() {
            Some(g) => {
                *out = *g;
                *seen = v;
                true
            }
            None => false,
        }
    }
}

#[derive(Clone, Copy)]
enum BandKind {
    LowShelf,
    Peak,
    HighShelf,
}

/// Биквад в транспонированной прямой форме II (RBJ cookbook), стерео.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: [f32; 2],
    z2: [f32; 2],
    active: bool,
}

impl Biquad {
    fn configure(&mut self, kind: BandKind, freq: f32, q: f32, gain_db: f32, fs: f32) {
        if gain_db.abs() < 0.05 || freq >= fs * 0.48 {
            self.active = false;
            self.z1 = [0.0; 2];
            self.z2 = [0.0; 2];
            return;
        }
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * PI * freq / fs;
        let (s, c) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match kind {
            BandKind::Peak => {
                let alpha = s / (2.0 * q);
                (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
            }
            BandKind::LowShelf => {
                let sa = 2.0 * a.sqrt() * (s / 2.0 * std::f32::consts::SQRT_2);
                (
                    a * ((a + 1.0) - (a - 1.0) * c + sa),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - sa),
                    (a + 1.0) + (a - 1.0) * c + sa,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - sa,
                )
            }
            BandKind::HighShelf => {
                let sa = 2.0 * a.sqrt() * (s / 2.0 * std::f32::consts::SQRT_2);
                (
                    a * ((a + 1.0) + (a - 1.0) * c + sa),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - sa),
                    (a + 1.0) - (a - 1.0) * c + sa,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - sa,
                )
            }
        };
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
        self.active = true;
    }

    #[inline(always)]
    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.b0 * x + self.z1[ch];
        self.z1[ch] = self.b1 * x - self.a1 * y + self.z2[ch];
        self.z2[ch] = self.b2 * x - self.a2 * y;
        y
    }

    /// Денормализованные числа в хвостах IIR-фильтров резко нагружают CPU — обнуляем их.
    #[inline]
    fn flush_denormals(&mut self) {
        for v in self.z1.iter_mut().chain(self.z2.iter_mut()) {
            if v.abs() < 1e-20 {
                *v = 0.0;
            }
        }
    }
}

const LIMIT_CEIL: f32 = 0.97;

#[inline(always)]
fn soft_clip(x: f32) -> f32 {
    let ax = x.abs();
    if ax <= LIMIT_CEIL {
        x
    } else {
        let over = (ax - LIMIT_CEIL) / (1.0 - LIMIT_CEIL);
        (LIMIT_CEIL + (1.0 - LIMIT_CEIL) * over.tanh()).copysign(x)
    }
}

pub struct DspChain {
    fs: f32,
    params: DspParams,
    seen: u64,
    bands: [Biquad; 10],
    preamp: f32,
    gain_now: f32,
    gain_k: f32,
    fade_now: f32,
    lim_env: f32,
    lim_rel: f32,
    bass_lp: f32,
    bass_k: f32,
    level_acc: f32,
    level_n: u32,
    level_out: Option<f32>,
}

impl DspChain {
    pub fn new(fs: f32) -> Self {
        let mut c = Self {
            fs,
            params: DspParams::default(),
            seen: 0,
            bands: [Biquad::default(); 10],
            preamp: 1.0,
            gain_now: 0.0,
            gain_k: 1.0 - (-1.0 / (0.025 * fs)).exp(),
            fade_now: 0.0,
            lim_env: 0.0,
            lim_rel: (-1.0 / (0.15 * fs)).exp(),
            bass_lp: 0.0,
            bass_k: 1.0 - (-2.0 * PI * 140.0 / fs).exp(),
            level_acc: 0.0,
            level_n: 0,
            level_out: None,
        };
        c.recompute();
        c
    }

    pub fn refresh(&mut self, ctl: &DspControl) {
        if ctl.try_load(&mut self.seen, &mut self.params) {
            self.recompute();
        }
    }

    fn recompute(&mut self) {
        let p = self.params;
        for (i, b) in self.bands.iter_mut().enumerate() {
            let kind = match i {
                0 => BandKind::LowShelf,
                9 => BandKind::HighShelf,
                _ => BandKind::Peak,
            };
            let db = if p.eq_enabled { p.eq_db[i].clamp(-24.0, 24.0) } else { 0.0 };
            b.configure(kind, EQ_FREQS[i], 1.41, db, self.fs);
        }
        self.preamp = if p.eq_enabled { 10f32.powf(p.preamp_db.clamp(-24.0, 12.0) / 20.0) } else { 1.0 };
    }

    /// Сбросить огибающую — следующий звук плавно нарастёт (после перемотки/смены трека).
    pub fn restart_fade(&mut self) {
        self.fade_now = 0.0;
    }

    /// Огибающая полностью закрыта — можно не читать буфер (пауза).
    #[inline]
    pub fn is_silent(&self) -> bool {
        self.fade_now <= 0.0
    }

    #[inline(always)]
    pub fn process(&mut self, mut l: f32, mut r: f32, fade_target: f32) -> (f32, f32) {
        let p = &self.params;

        // огибающая паузы/перемотки
        if self.fade_now != fade_target {
            let step = if p.fade_ms <= 0.5 { 1.0 } else { 1000.0 / (p.fade_ms * self.fs) };
            if self.fade_now < fade_target {
                self.fade_now = (self.fade_now + step).min(fade_target);
            } else {
                self.fade_now = (self.fade_now - step).max(fade_target);
            }
        }

        if p.eq_enabled {
            l *= self.preamp;
            r *= self.preamp;
            for b in self.bands.iter_mut() {
                if b.active {
                    l = b.process(0, l);
                    r = b.process(1, r);
                }
            }
        }

        if p.mono {
            let m = 0.5 * (l + r);
            l = m;
            r = m;
        }
        if p.balance > 0.0 {
            l *= 1.0 - p.balance.min(1.0);
        } else if p.balance < 0.0 {
            r *= 1.0 + p.balance.max(-1.0);
        }

        self.gain_now += (p.gain - self.gain_now) * self.gain_k;
        // квадратичная огибающая звучит естественнее линейной
        let g = self.gain_now * self.fade_now * self.fade_now;
        l *= g;
        r *= g;

        if p.limiter {
            let peak = l.abs().max(r.abs());
            if peak > self.lim_env {
                self.lim_env = peak;
            } else {
                self.lim_env = peak + (self.lim_env - peak) * self.lim_rel;
            }
            if self.lim_env > LIMIT_CEIL {
                let gr = LIMIT_CEIL / self.lim_env;
                l *= gr;
                r *= gr;
            }
            l = soft_clip(l);
            r = soft_clip(r);
        } else {
            l = l.clamp(-1.0, 1.0);
            r = r.clamp(-1.0, 1.0);
        }

        // уровень баса для «пульсации» обложки
        self.bass_lp += (0.5 * (l + r) - self.bass_lp) * self.bass_k;
        self.level_acc += self.bass_lp * self.bass_lp;
        self.level_n += 1;
        if self.level_n >= 1024 {
            self.level_out = Some((self.level_acc / self.level_n as f32).sqrt());
            self.level_acc = 0.0;
            self.level_n = 0;
        }

        (l, r)
    }

    pub fn end_block(&mut self) {
        for b in self.bands.iter_mut() {
            b.flush_denormals();
        }
        if self.bass_lp.abs() < 1e-20 {
            self.bass_lp = 0.0;
        }
    }

    pub fn take_level(&mut self) -> Option<f32> {
        self.level_out.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_eq_is_transparent_at_unity_gain() {
        let ctl = DspControl::new(DspParams { fade_ms: 0.0, limiter: false, ..Default::default() });
        let mut c = DspChain::new(48000.0);
        c.refresh(&ctl);
        // прогреваем сглаживание громкости
        for _ in 0..48000 {
            c.process(0.0, 0.0, 1.0);
        }
        let (l, r) = c.process(0.5, -0.25, 1.0);
        assert!((l - 0.5).abs() < 1e-3 && (r + 0.25).abs() < 1e-3, "{l} {r}");
    }

    #[test]
    fn limiter_never_exceeds_full_scale_at_200_percent() {
        let ctl = DspControl::new(DspParams { gain: 2.0, fade_ms: 0.0, ..Default::default() });
        let mut c = DspChain::new(48000.0);
        c.refresh(&ctl);
        let mut max = 0f32;
        for i in 0..96000 {
            let x = (i as f32 * 0.05).sin() * 0.95;
            let (l, r) = c.process(x, x, 1.0);
            max = max.max(l.abs()).max(r.abs());
        }
        assert!(max <= 1.0, "peak {max}");
        assert!(max > 0.9, "лимитер не должен душить сигнал: {max}");
    }

    fn rms_of_tone(freq: f32, eq_db: [f32; 10], eq_enabled: bool) -> f32 {
        let ctl = DspControl::new(DspParams { eq_enabled, eq_db, fade_ms: 0.0, limiter: false, gain: 0.25, ..Default::default() });
        let mut c = DspChain::new(48000.0);
        c.refresh(&ctl);
        let mut acc = 0.0;
        for i in 0..48000 {
            let x = (2.0 * PI * freq * i as f32 / 48000.0).sin() * 0.5;
            let (l, _) = c.process(x, x, 1.0);
            if i > 24000 {
                acc += l * l;
            }
        }
        (acc / 23999.0).sqrt()
    }

    #[test]
    fn eq_bands_boost_and_cut() {
        let mut bass = [0.0; 10];
        bass[0] = 9.0;
        bass[1] = 9.0;
        let flat = rms_of_tone(50.0, [0.0; 10], false);
        let boosted = rms_of_tone(50.0, bass, true);
        assert!(boosted / flat > 2.0, "буст баса: {flat} → {boosted}");

        let mut cut = [0.0; 10];
        cut[5] = -12.0;
        let flat = rms_of_tone(1000.0, [0.0; 10], false);
        let cutted = rms_of_tone(1000.0, cut, true);
        assert!(cutted / flat < 0.35, "срез 1 кГц: {flat} → {cutted}");
        // соседняя полоса почти не задета
        let far = rms_of_tone(8000.0, cut, true) / rms_of_tone(8000.0, [0.0; 10], false);
        assert!(far > 0.9, "8 кГц: {far}");
    }
}
