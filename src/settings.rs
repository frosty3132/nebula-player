//! Пользовательские настройки (звук, воспроизведение, текст, интерфейс, кэш).
//! Хранятся в `settings.json`; любые незнакомые/отсутствующие поля получают значения по умолчанию.

use crate::audio::DspParams;
use crate::paths::{paths, read_json, write_json};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn next(self) -> Self {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StreamQuality {
    /// AAC 160 кбит/с
    #[default]
    High,
    /// AAC 96 кбит/с — экономия трафика
    Low,
    /// MP3 128 кбит/с, если SoundCloud его ещё отдаёт
    Mp3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WindowEffect {
    #[default]
    None,
    Mica,
    Acrylic,
    Blur,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VolumeCurve {
    /// Квадратичная до 100% (как в большинстве плееров), линейная выше
    #[default]
    Perceptual,
    Linear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LyricsAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub width: f64,
    pub height: f64,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self { width: 1320.0, height: 820.0, x: None, y: None, maximized: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    // ─── звук ───
    pub volume: f32,
    pub max_volume: f32,
    pub muted: bool,
    pub volume_curve: VolumeCurve,
    pub volume_step: f32,
    pub limiter: bool,
    pub eq_enabled: bool,
    pub eq_bands: [f32; 10],
    pub eq_preamp: f32,
    pub eq_preset: String,
    pub balance: f32,
    pub mono: bool,
    pub fade_ms: f32,
    pub output_device: Option<String>,
    pub quality: StreamQuality,
    pub read_ahead_s: u32,
    // ─── воспроизведение ───
    pub autoplay_related: bool,
    pub preload_next: bool,
    /// Плавный переход между треками (кроссфейд), секунды; 0 — выключен.
    pub crossfade_s: f32,
    pub remember_session: bool,
    pub skip_previews: bool,
    /// Превью и недоступные треки SoundCloud играть полной версией с YouTube (yt-dlp).
    pub yt_replace_previews: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub seek_step_s: f32,
    pub prev_restart_s: f32,
    // ─── текст песен ───
    pub lyrics_enabled: bool,
    pub lyrics_auto_open: bool,
    pub lyrics_font: f32,
    pub lyrics_align: LyricsAlign,
    pub lyrics_blur: bool,
    pub lyrics_karaoke: bool,
    pub lyrics_offset_ms: i32,
    pub lyrics_plain_fallback: bool,
    pub lyrics_width: f32,
    pub lyrics_autoscroll: bool,
    pub lyrics_translit: bool,
    // ─── интерфейс ───
    pub theme: String,
    /// Один раз включена тема «Монохром» (переход на стиль «Сцена»).
    pub style_mono_applied: bool,
    /// Пользовательский фон обложки на сцене (путь к изображению / GIF).
    #[serde(default)]
    pub stage_cover_bg: Option<String>,
    /// Последний выбранный фон — им можно снова заменить пластинку одним нажатием.
    #[serde(default)]
    pub stage_cover_bg_last: Option<String>,
    pub ui_scale: f32,
    pub animations: bool,
    pub anim_speed: f32,
    /// Заставка при запуске: логотип и каскадное появление интерфейса.
    pub intro_anim: bool,
    pub window_effect: WindowEffect,
    pub always_on_top: bool,
    pub pause_bg_unfocused: bool,
    pub low_power: bool,
    pub dynamic_accent: bool,
    pub beat_glow: bool,
    /// Визуализатор спектра (как cava) в планете на главной.
    pub visualizer: bool,
    /// «Рыбий глаз»: строки у верхнего и нижнего края списка сжимаются и наклоняются.
    pub fisheye: bool,
    pub discord_rpc_enabled: bool,
    pub discord_client_id: String,
    pub show_track_numbers: bool,
    pub confirm_clear_queue: bool,
    pub window: WindowState,
    // ─── кэш ───
    pub image_cache_mb: u32,
    pub api_cache_min: u32,
    // ─── локальная музыка ───
    pub music_folders: Vec<String>,
    // ─── состояние панелей ───
    pub lyrics_open: bool,
    /// Панель текста закрыта вручную: автооткрытие её не возвращает, пока
    /// пользователь сам не откроет панель снова.
    pub lyrics_dismissed: bool,
    pub queue_open: bool,
    /// Нижний плеер свёрнут в островок в заголовке окна.
    pub player_collapsed: bool,
    /// Свёрнутый плеер — островок на рабочем столе поверх всех окон Windows.
    pub island_desktop: bool,
    /// Крестик прячет Nebula в трей (музыка играет дальше), а не закрывает.
    pub close_to_tray: bool,
    /// Город для погоды в островке (пусто — погода не показывается).
    pub weather_city: String,
    /// Папка для скачанных треков (пусто — системная «Загрузки»).
    pub download_dir: Option<String>,
    /// Спрашивать, куда сохранить, при каждом скачивании.
    pub download_ask: bool,
    /// Оформление часов в «Сцене»: ring, digital, analog, flip, bar.
    pub clock_style: String,
    pub sidebar_collapsed: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 0.8,
            max_volume: 2.0,
            muted: false,
            volume_curve: VolumeCurve::Perceptual,
            volume_step: 0.05,
            limiter: true,
            eq_enabled: false,
            eq_bands: [0.0; 10],
            eq_preamp: 0.0,
            eq_preset: "flat".into(),
            balance: 0.0,
            mono: false,
            fade_ms: 140.0,
            output_device: None,
            quality: StreamQuality::High,
            read_ahead_s: 240,
            autoplay_related: true,
            preload_next: true,
            crossfade_s: 6.0,
            remember_session: true,
            skip_previews: false,
            yt_replace_previews: true,
            shuffle: false,
            repeat: RepeatMode::Off,
            seek_step_s: 5.0,
            prev_restart_s: 3.0,
            lyrics_enabled: true,
            lyrics_auto_open: true,
            lyrics_font: 22.0,
            lyrics_align: LyricsAlign::Center,
            lyrics_blur: true,
            lyrics_karaoke: true,
            lyrics_offset_ms: 0,
            lyrics_plain_fallback: true,
            lyrics_width: 380.0,
            lyrics_autoscroll: true,
            lyrics_translit: false,
            theme: "mono".into(),
            style_mono_applied: true,
            stage_cover_bg: None,
            stage_cover_bg_last: None,
            ui_scale: 1.0,
            animations: true,
            anim_speed: 1.0,
            intro_anim: true,
            window_effect: WindowEffect::None,
            always_on_top: false,
            pause_bg_unfocused: true,
            low_power: false,
            dynamic_accent: false,
            beat_glow: true,
            visualizer: true,
            fisheye: true,
            discord_rpc_enabled: true,
            discord_client_id: String::new(),
            show_track_numbers: true,
            confirm_clear_queue: false,
            window: WindowState::default(),
            image_cache_mb: 400,
            api_cache_min: 30,
            music_folders: default_music_folder(),
            lyrics_open: true,
            lyrics_dismissed: false,
            queue_open: false,
            player_collapsed: false,
            island_desktop: true,
            close_to_tray: true,
            weather_city: String::new(),
            download_dir: None,
            download_ask: true,
            clock_style: "ring".into(),
            sidebar_collapsed: false,
        }
    }
}

fn default_music_folder() -> Vec<String> {
    dirs::audio_dir().map(|p| vec![p.display().to_string()]).unwrap_or_default()
}

impl Settings {
    pub fn load() -> Self {
        read_json::<Settings>(&paths().config.join("settings.json")).unwrap_or_default().sanitized()
    }

    pub fn save(&self) {
        if let Err(e) = write_json(&paths().config.join("settings.json"), self) {
            tracing::warn!("не удалось сохранить настройки: {e}");
        }
    }

    /// Защита от ручной правки файла: зажимаем значения в разумные рамки.
    pub fn sanitized(mut self) -> Self {
        self.max_volume = self.max_volume.clamp(1.0, 4.0);
        self.volume = self.volume.clamp(0.0, self.max_volume);
        self.volume_step = self.volume_step.clamp(0.01, 0.25);
        self.ui_scale = self.ui_scale.clamp(0.6, 1.8);
        self.anim_speed = self.anim_speed.clamp(0.25, 3.0);
        self.crossfade_s = self.crossfade_s.clamp(0.0, 12.0);
        self.lyrics_font = self.lyrics_font.clamp(12.0, 64.0);
        self.lyrics_width = self.lyrics_width.clamp(260.0, 900.0);
        self.fade_ms = self.fade_ms.clamp(0.0, 2000.0);
        self.read_ahead_s = self.read_ahead_s.clamp(30, 1800);
        self.balance = self.balance.clamp(-1.0, 1.0);
        for b in self.eq_bands.iter_mut() {
            *b = b.clamp(-15.0, 15.0);
        }
        self.window.width = self.window.width.clamp(860.0, 8000.0);
        self.window.height = self.window.height.clamp(560.0, 8000.0);
        self
    }

    /// Линейный коэффициент громкости. 200% = ×2 (+6 дБ).
    pub fn gain(&self) -> f32 {
        if self.muted {
            return 0.0;
        }
        let v = self.volume.clamp(0.0, self.max_volume);
        match self.volume_curve {
            VolumeCurve::Linear => v,
            VolumeCurve::Perceptual => {
                if v <= 1.0 {
                    v * v
                } else {
                    v
                }
            }
        }
    }

    pub fn dsp(&self) -> DspParams {
        DspParams {
            gain: self.gain(),
            eq_enabled: self.eq_enabled,
            eq_db: self.eq_bands,
            preamp_db: self.eq_preamp,
            balance: self.balance,
            mono: self.mono,
            limiter: self.limiter,
            fade_ms: if self.low_power { self.fade_ms.min(60.0) } else { self.fade_ms },
        }
    }
}

/// Пресеты эквалайзера: (id, название, полосы 31 Гц … 16 кГц).
pub const EQ_PRESETS: &[(&str, &str, [f32; 10])] = &[
    ("flat", "Ровно", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("bass", "Бас", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("superbass", "Супербас", [9.0, 8.0, 6.0, 3.0, 0.0, -1.0, -1.0, 0.0, 0.0, 0.0]),
    ("treble", "Высокие", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("vocal", "Вокал", [-2.0, -2.0, -1.0, 1.0, 3.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("electronic", "Электроника", [5.0, 4.0, 1.0, 0.0, -2.0, 1.0, 0.0, 1.0, 4.0, 5.0]),
    ("rock", "Рок", [4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 1.0, 2.0, 3.0, 4.0]),
    ("hiphop", "Хип-хоп", [5.0, 4.0, 1.0, 2.0, -1.0, -1.0, 1.0, 0.0, 2.0, 3.0]),
    ("lofi", "Lo-Fi", [2.0, 3.0, 2.0, 0.0, -1.0, -2.0, -3.0, -4.0, -6.0, -8.0]),
    ("night", "Ночной", [3.0, 2.0, 1.0, 0.0, 0.0, -1.0, -2.0, -3.0, -4.0, -5.0]),
    ("space", "Космос", [4.0, 3.0, 1.0, 0.0, -1.0, 0.0, 2.0, 3.0, 4.0, 3.0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_curve() {
        let mut s = Settings { volume: 1.0, ..Default::default() };
        assert!((s.gain() - 1.0).abs() < 1e-6);
        s.volume = 2.0;
        assert!((s.gain() - 2.0).abs() < 1e-6, "200% = ×2");
        s.volume = 0.5;
        assert!((s.gain() - 0.25).abs() < 1e-6);
        s.muted = true;
        assert_eq!(s.gain(), 0.0);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let s: Settings = serde_json::from_str(r#"{"volume": 1.5, "something_new": 1}"#).unwrap();
        assert_eq!(s.volume, 1.5);
        assert_eq!(s.theme, "mono");
    }
}
