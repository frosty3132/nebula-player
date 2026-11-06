//! Аудиодвижок Nebula.
//!
//! Загрузка (файлы / HTTP / HLS) → декодирование (symphonia) → ресемплинг →
//! lock-free кольцевой буфер → DSP в колбэке cpal (EQ, громкость до 400%, лимитер).

mod decode;
mod dsp;
mod hls;
mod output;
mod player;
mod resample;
mod source;
mod spectrum;
mod ts;

pub use dsp::{DspParams, EQ_FREQS};
pub use player::{MediaSpec, PlaybackState, Player, PlayerEvent};
pub use spectrum::BANDS as SPECTRUM_BANDS;
