//! Потоковый ресемплер на кубической интерполяции Катмулла–Рома.
//! Дешёвый (4 умножения на отсчёт) и для повышения 44.1 → 48 кГц на слух
//! неотличим от «тяжёлых» sinc-фильтров. Буферы переиспользуются.

pub struct Resampler {
    step: f64,
    pos: f64,
    buf: Vec<[f32; 2]>,
    passthrough: bool,
}

#[inline(always)]
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    y1 + 0.5 * t * (y2 - y0 + t * (2.0 * y0 - 5.0 * y1 + 4.0 * y2 - y3 + t * (3.0 * (y1 - y2) + y3 - y0)))
}

impl Resampler {
    pub fn new(src_rate: u32, dst_rate: u32) -> Self {
        Self {
            step: src_rate.max(1) as f64 / dst_rate.max(1) as f64,
            pos: 1.0,
            // один кадр тишины слева служит историей для первой интерполяции
            buf: vec![[0.0; 2]],
            passthrough: src_rate == dst_rate,
        }
    }

    /// Добавляет стерео-кадры источника и дописывает интерливный результат в `out`.
    pub fn process(&mut self, input: &[[f32; 2]], out: &mut Vec<f32>) {
        if self.passthrough {
            out.reserve(input.len() * 2);
            for f in input {
                out.push(f[0]);
                out.push(f[1]);
            }
            return;
        }
        self.buf.extend_from_slice(input);
        let buf = &self.buf;
        out.reserve(((input.len() as f64 / self.step) as usize + 2) * 2);
        loop {
            let i = self.pos as usize;
            if i + 2 >= buf.len() {
                break;
            }
            let t = (self.pos - i as f64) as f32;
            let (a, b, c, d) = (buf[i - 1], buf[i], buf[i + 1], buf[i + 2]);
            out.push(cubic(a[0], b[0], c[0], d[0], t));
            out.push(cubic(a[1], b[1], c[1], d[1], t));
            self.pos += self.step;
        }
        let keep_from = (self.pos as usize).saturating_sub(1).min(self.buf.len());
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_length_matches_ratio() {
        let mut r = Resampler::new(44100, 48000);
        let input = vec![[0.25f32, -0.25]; 44100];
        let mut out = Vec::new();
        for chunk in input.chunks(1000) {
            r.process(chunk, &mut out);
        }
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() < 8, "frames = {frames}");
        // постоянный сигнал должен остаться постоянным (кроме самого начала)
        assert!(out[200..].chunks(2).all(|f| (f[0] - 0.25).abs() < 1e-4 && (f[1] + 0.25).abs() < 1e-4));
    }

    #[test]
    fn passthrough_is_exact() {
        let mut r = Resampler::new(48000, 48000);
        let mut out = Vec::new();
        r.process(&[[0.1, 0.2], [0.3, 0.4]], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3, 0.4]);
    }
}
