//! Nebula — галактический музыкальный плеер для SoundCloud и локальной музыки.
//!
//! Интерфейс — компоненты Dioxus на Rust, отрисовка — системный WebView2;
//! звук, сеть, кэш и тексты — нативный Rust-код (symphonia, cpal, reqwest, tokio).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod discord;
mod import;
mod library;
mod lyrics;
mod media;
mod model;
mod paths;
mod sc_login;
mod services;
mod settings;
mod soundcloud;
mod spotify;
mod theme;
mod ui;
mod yandex;
mod youtube;

use dioxus::desktop::tao::dpi::{LogicalSize, PhysicalPosition};
use dioxus::desktop::tao::window::Icon;
use dioxus::desktop::{Config, WindowBuilder};

const BASE_CSS: &str = include_str!("../assets/style.css");
const LOGIN_CSS: &str = include_str!("../assets/login.css");

fn init_logging(p: &paths::Paths) {
    let file = std::fs::OpenOptions::new().create(true).append(true).open(p.cache.join("nebula.log"));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,wry=warn,tao=warn,dioxus=warn,dioxus_core=warn,reqwest=warn,hyper=warn"));
    if let Ok(file) = file {
        let _ = tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(std::sync::Mutex::new(file)).try_init();
    }
}

/// Иконка окна рисуется кодом: планета с кольцом в цветах темы «Галактика».
fn app_icon() -> Option<Icon> {
    let (rgba, w, h) = app_icon_rgba();
    Icon::from_rgba(rgba, w, h).ok()
}

/// Пиксели иконки (RGBA, 64×64) — для окна и значка в трее.
pub fn app_icon_rgba() -> (Vec<u8>, u32, u32) {
    // Логотип из плеера (планета с кольцом, как LOGO_SVG в координатах 64×64)
    // на тёмном квадрате. Рисуем с запасом по размеру и сглаживанием по
    // расстоянию до края фигур.
    const S: usize = 128;
    const BG: [f32; 3] = [18.0, 18.0, 18.0];
    let mut rgba = vec![0u8; S * S * 4];
    // логотип занимает ~74% значка
    let k = S as f32 * 0.74 / 64.0;
    let off = (S as f32 - 64.0 * k) / 2.0;
    let (sn, cs) = (-18f32).to_radians().sin_cos();
    let cov = |d: f32| (0.5 - d * k).clamp(0.0, 1.0); // d < 0 — внутри фигуры
    for y in 0..S {
        for x in 0..S {
            // координаты пикселя в системе логотипа (0..64)
            let lx = (x as f32 + 0.5 - off) / k;
            let ly = (y as f32 + 0.5 - off) / k;
            let mut px = BG;
            let mut over = |col: [f32; 3], a: f32| {
                for i in 0..3 {
                    px[i] = px[i] * (1.0 - a) + col[i] * a;
                }
            };
            // кольцо в повёрнутой на −18° системе вокруг (32, 34)
            let (dx, dy) = (lx - 32.0, ly - 34.0);
            let (rx, ry) = (dx * cs + dy * sn, -dx * sn + dy * cs);
            let e = ((rx / 29.0).powi(2) + (ry / 9.0).powi(2)).sqrt();
            // приблизительное расстояние до эллипса (в единицах логотипа)
            let ring_d = (e - 1.0).abs() * 9.0 * (1.0 + (rx / 29.0).powi(2) * 2.2) - 1.5;
            let ring = cov(ring_d);
            // задняя половина кольца — приглушённая, за планетой
            if ring > 0.0 && ry < 0.0 {
                over([200.0, 200.0, 200.0], ring * 0.45);
            }
            // планета: светлый градиент и блик
            let pr = ((lx - 32.0).powi(2) + (ly - 32.0).powi(2)).sqrt();
            let planet = cov(pr - 15.0);
            if planet > 0.0 {
                let t = (((lx - 17.0) + (ly - 17.0)) / 60.0).clamp(0.0, 1.0);
                let base = 250.0 - 70.0 * t;
                let shine = (1.0 - ((lx - 26.5).powi(2) + (ly - 24.5).powi(2)).sqrt() / 13.0).max(0.0) * 0.5;
                let v = base + (255.0 - base) * shine;
                over([v, v, v], planet);
            }
            // передняя половина кольца — поверх планеты
            if ring > 0.0 && ry >= 0.0 {
                over([245.0, 245.0, 245.0], ring);
            }
            // звёзды
            for (sx, sy, r, a) in [(52.0f32, 12.0f32, 1.6f32, 1.0f32), (10.0, 18.0, 1.0, 0.7)] {
                let d = ((lx - sx).powi(2) + (ly - sy).powi(2)).sqrt() - r;
                let c = cov(d);
                if c > 0.0 {
                    over([255.0, 255.0, 255.0], c * a);
                }
            }
            let i = (y * S + x) * 4;
            for c in 0..3 {
                rgba[i + c] = px[c].clamp(0.0, 255.0) as u8;
            }
            rgba[i + 3] = 255;
        }
    }
    (rgba, S as u32, S as u32)
}

/// Тёмные системные меню (контекстное меню значка в трее), как у тёмных
/// приложений Windows 11. Функции uxtheme не документированы и берутся по
/// номеру; если их нет (старая Windows), меню просто останутся светлыми.
fn dark_menus() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();
        let lib = LoadLibraryW(name.as_ptr());
        if lib.is_null() {
            return;
        }
        // 135 — SetPreferredAppMode(ForceDark = 2), 136 — FlushMenuThemes
        if let Some(f) = GetProcAddress(lib, 135usize as *const u8) {
            let set_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
            set_mode(2);
        }
        if let Some(f) = GetProcAddress(lib, 136usize as *const u8) {
            let flush: unsafe extern "system" fn() = std::mem::transmute(f);
            flush();
        }
    }
}

/// Параметры WebView2. Одинаковые для всех окон приложения (главного и
/// островка): окна с общей папкой данных WebView2 обязаны совпадать в них.
pub fn webview_args() -> String {
    #[allow(unused_mut)]
    let mut args = String::from("--autoplay-policy=no-user-gesture-required --disable-features=msSmartScreenProtection");
    // Только для отладочной сборки: доступ к DevTools Protocol для автотестов интерфейса.
    #[cfg(debug_assertions)]
    if let Ok(port) = std::env::var("NEBULA_DEVTOOLS_PORT") {
        args.push_str(&format!(" --remote-debugging-port={port} --remote-allow-origins=http://127.0.0.1:{port}"));
    }
    args
}

fn main() {
    let p = paths::init();
    init_logging(p);
    tracing::info!("Nebula {} запускается; данные: {}", env!("CARGO_PKG_VERSION"), p.config.display());

    let settings = settings::Settings::load();
    dark_menus();

    // Один многопоточный рантайм на всё приложение: сеть, кэш, загрузка аудио.
    // Dioxus подхватывает его через текущий контекст.
    let rt: &'static tokio::runtime::Runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(3)
            .thread_name("nebula-io")
            .enable_all()
            .build()
            .expect("не удалось создать tokio-рантайм"),
    ));
    let _guard = rt.enter();
    services::init(rt.handle().clone(), &settings);

    let w = settings.window;
    let mut wb = WindowBuilder::new()
        .with_title("Nebula")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(w.width, w.height))
        .with_min_inner_size(LogicalSize::new(860.0, 560.0))
        .with_maximized(w.maximized)
        .with_window_icon(app_icon());
    if let (Some(x), Some(y)) = (w.x, w.y) {
        if (-4000..12000).contains(&x) && (-200..8000).contains(&y) {
            wb = wb.with_position(PhysicalPosition::new(x, y));
        }
    }
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowBuilderExtWindows;
        wb = wb.with_undecorated_shadow(false);
    }

    let browser_args = webview_args();

    let cfg = Config::new()
        .with_window(wb)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<meta name=\"color-scheme\" content=\"dark light\"><style>{BASE_CSS}{LOGIN_CSS}</style>"))
        .with_data_directory(&p.webview)
        .with_disable_context_menu(!cfg!(debug_assertions))
        .with_windows_browser_args(browser_args)
        .with_tray_icon_show_window_on_click(false)
        // окно входа в SoundCloud живёт в этом же цикле событий
        .with_custom_event_handler(|event, target| sc_login::pump(event, target));

    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(ui::App);
}

#[cfg(test)]
mod icon_tests {
    /// Иконка рисуется кодом — проверяем размер и непрозрачный тёмный угол.
    #[test]
    fn icon_is_square_and_dark_cornered() {
        let (px, w, h) = super::app_icon_rgba();
        assert_eq!((w, h), (128, 128));
        assert_eq!(px.len(), 128 * 128 * 4);
        assert_eq!(&px[0..4], &[18, 18, 18, 255]);
        // в центре — светлая планета
        let c = (64 * 128 + 64) * 4;
        assert!(px[c] > 180);
        if let Ok(p) = std::env::var("NEBULA_ICON_DUMP") {
            let _ = std::fs::write(p, &px);
        }
    }
}
