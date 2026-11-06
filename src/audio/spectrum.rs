//! Анализатор спектра для визуализатора в духе cava: банк полосовых
//! биквадратных фильтров (по трети октавы, 50 Гц – 12 кГц). Считается в
//! колбэке вывода только пока визуализатор показан на экране, результат
//! (0..1 на полосу) кладётся в атомики — интерфейс читает их без блокировок.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub const BANDS: usize = 24;
const F_LO: f32 = 50.0;
const F_HI: f32 = 12_000.0;
const Q: f32 = 4.0;

/// Общие данные анализатора: включён ли и последние значения полос.
pub struct Spectrum {
    pub enabled: AtomicBool,
    bands: [AtomicU32; BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self { enabled: AtomicBool::new(false), bands: std::array::from_fn(|_| AtomicU32::new(0)) }
    }
}

impl Spectrum {
    pub fn read(&self) -> [f32; BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.bands[i].load(Ordering::Relaxed)))
    }

    fn clear(&self) {
        for b in &self.bands {
            b.store(0, Ordering::Relaxed);
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Bp {
    b0: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Bp {
    /// Полосовой фильтр RBJ (усиление в центре полосы 0 дБ).
    fn new(f0: f32, rate: f32) -> Self {
        let w0 = std::f32::consts::TAU * f0 / rate;
        let alpha = w0.sin() / (2.0 * Q);
        let a0 = 1.0 + alpha;
        Self { b0: alpha / a0, b2: -alpha / a0, a1: -2.0 * w0.cos() / a0, a2: (1.0 - alpha) / a0, z1: 0.0, z2: 0.0 }
    }

    #[inline(always)]
    fn run(&mut self, x: f32) -> f32 {
        // транспонированная прямая форма II
        let y = self.b0 * x + self.z1;
        self.z1 = -self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Состояние анализатора внутри колбэка (без аллокаций в работе).
pub struct Analyzer {
    // по два звена на полосу (4-й порядок): полосы не «подсвечивают» соседей
    filters: [[Bp; 2]; BANDS],
    energy: [f32; BANDS],
    shown: [f32; BANDS],
    samples: u32,
    was_on: bool,
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let rate = rate as f32;
        let hi = F_HI.min(rate * 0.45);
        let filters = std::array::from_fn(|i| {
            let t = i as f32 / (BANDS - 1) as f32;
            let b = Bp::new(F_LO * (hi / F_LO).powf(t), rate);
            [b, b]
        });
        Self { filters, energy: [0.0; BANDS], shown: [0.0; BANDS], samples: 0, was_on: false }
    }

    #[inline(always)]
    pub fn push(&mut self, x: f32) {
        for (f, e) in self.filters.iter_mut().zip(self.energy.iter_mut()) {
            let y0 = f[0].run(x);
            let y = f[1].run(y0);
            *e += y * y;
        }
        self.samples += 1;
    }

    /// Конец блока: перевести энергию в 0..1 со «спадом» как в cava и опубликовать.
    pub fn publish(&mut self, out: &Spectrum, on: bool) {
        if !on {
            if self.was_on {
                self.was_on = false;
                self.shown = [0.0; BANDS];
                self.filters.iter_mut().flatten().for_each(|f| (f.z1, f.z2) = (0.0, 0.0));
                out.clear();
            }
            self.samples = 0;
            self.energy = [0.0; BANDS];
            return;
        }
        self.was_on = true;
        let n = self.samples.max(1) as f32;
        for i in 0..BANDS {
            let rms = (self.energy[i] / n).sqrt();
            let db = 20.0 * (rms + 1e-9).log10();
            let v = ((db + 60.0) / 48.0).clamp(0.0, 1.0);
            // быстрый подъём, плавное падение
            let cur = self.shown[i];
            self.shown[i] = if v > cur { cur + (v - cur) * 0.7 } else { (cur * 0.9).max(v) };
            out.bands[i].store(self.shown[i].to_bits(), Ordering::Relaxed);
            self.energy[i] = 0.0;
        }
        self.samples = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_lights_up_its_band() {
        let rate = 48_000;
        let mut a = Analyzer::new(rate);
        let s = Spectrum::default();
        // синус 1 кГц, 0,2 с
        for k in 0..(rate / 5) {
            a.push((std::f32::consts::TAU * 1000.0 * k as f32 / rate as f32).sin() * 0.5);
            if k % 480 == 479 {
                a.publish(&s, true);
            }
        }
        let v = s.read();
        let peak = (0..BANDS).max_by(|&x, &y| v[x].total_cmp(&v[y])).unwrap();
        let f = F_LO * (F_HI / F_LO).powf(peak as f32 / (BANDS - 1) as f32);
        assert!((700.0..1500.0).contains(&f), "пик на {f} Гц");
        assert!(v[peak] > 0.6 && v[0] < 0.2, "{v:?}");
    }
}
