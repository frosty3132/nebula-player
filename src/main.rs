//! Nebula — галактический музыкальный плеер для SoundCloud и локальной музыки.
//!
//! Интерфейс — компоненты Dioxus на Rust, отрисовка — системный WebView2;
//! звук, сеть, кэш и тексты — нативный Rust-код (symphonia, cpal, reqwest, tokio).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod library;
mod lyrics;
mod media;
mod model;
mod paths;
mod sc_login;
mod services;
mod settings;
mod soundcloud;
mod theme;
mod ui;

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
    const S: usize = 64;
    let mut rgba = vec![0u8; S * S * 4];
    let c = S as f32 / 2.0;
    for y in 0..S {
        for x in 0..S {
            let (fx, fy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let i = (y * S + x) * 4;
            let mut px = [0f32; 4];
            // планета: градиент фиолетовый → бирюзовый
            let r = (fx * fx + fy * fy).sqrt();
            if r < 17.0 {
                let t = ((fx + fy) / 34.0 + 0.5).clamp(0.0, 1.0);
                let a = (17.0 - r).clamp(0.0, 1.0);
                px = [139.0 + (34.0 - 139.0) * t, 92.0 + (211.0 - 92.0) * t, 246.0 + (238.0 - 246.0) * t, 255.0 * a];
                let shine = (1.0 - ((fx + 6.0).powi(2) + (fy + 6.0).powi(2)).sqrt() / 14.0).max(0.0) * 0.55;
                for k in 0..3 {
                    px[k] += (255.0 - px[k]) * shine;
                }
            }
            // кольцо: эллипс, повёрнутый на -18°
            let (s, co) = (-0.314f32).sin_cos();
            let (rx, ry) = (fx * co - fy * s, fx * s + fy * co);
            let e = (rx / 29.0).powi(2) + (ry / 9.0).powi(2);
            let ring = (1.0 - ((e - 1.0).abs() / 0.12)).clamp(0.0, 1.0);
            let front = ry > 0.0 || r >= 17.0;
            if ring > 0.0 && front {
                let a = 255.0 * ring;
                let blend = a / 255.0;
                for k in 0..3 {
                    let col = [196.0, 181.0, 253.0][k];
                    px[k] = px[k] * (1.0 - blend) + col * blend;
                }
                px[3] = px[3].max(a);
            }
            for k in 0..4 {
                rgba[i + k] = px[k].clamp(0.0, 255.0) as u8;
            }
        }
    }
    Icon::from_rgba(rgba, S as u32, S as u32).ok()
}

fn main() {
    let p = paths::init();
    init_logging(p);
    tracing::info!("Nebula {} запускается; данные: {}", env!("CARGO_PKG_VERSION"), p.config.display());

    let settings = settings::Settings::load();

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

    #[allow(unused_mut)]
    let mut browser_args = String::from("--autoplay-policy=no-user-gesture-required --disable-features=msSmartScreenProtection");
    // Только для отладочной сборки: доступ к DevTools Protocol для автотестов интерфейса.
    #[cfg(debug_assertions)]
    if let Ok(port) = std::env::var("NEBULA_DEVTOOLS_PORT") {
        browser_args.push_str(&format!(" --remote-debugging-port={port} --remote-allow-origins=http://127.0.0.1:{port}"));
    }

    let cfg = Config::new()
        .with_window(wb)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<meta name=\"color-scheme\" content=\"dark light\"><style>{BASE_CSS}{LOGIN_CSS}</style>"))
        .with_data_directory(&p.webview)
        .with_disable_context_menu(!cfg!(debug_assertions))
        .with_windows_browser_args(browser_args)
        // окно входа в SoundCloud живёт в этом же цикле событий; обработчики Dioxus
        // получают события только своего окна, поэтому подключаемся на уровне цикла
        .with_custom_event_handler(|event, target| sc_login::pump(event, target));

    dioxus::LaunchBuilder::desktop().with_cfg(cfg).launch(ui::App);
}
