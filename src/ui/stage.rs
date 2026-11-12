//! «Сцена» — полноэкранный режим «сейчас играет»: слева колонка карточек
//! (обложка, плеер, часы и листаемый виджет), справа крупный текст песни.
//! Верхняя полоса — название и часы, плашка текущего трека по центру,
//! переключатель «Медиатека / Сцена» и кнопки окна.

use super::actions::*;
use super::chrome::{close_app, minimize_app, toggle_maximized_quiet};
use super::icons::{Icon, LOGO_SVG};
use super::player_bar::{Controls, Cover};
use super::state::*;
use super::tracks::PlayingBars;
use super::widgets::IconBtn;
use crate::audio::{PlaybackState, EQ_FREQS};
use crate::model::{fmt_listening, ArtSize};
use crate::services::svc;
use dioxus::desktop::window;
use dioxus::prelude::*;
use std::time::Duration;

/// Закрыть сцену (и полноэкранное окно, если оно было включено из неё).
pub fn close_stage(st: AppState) {
    if window().fullscreen().is_some() {
        window().set_fullscreen(false);
    }
    let mut f = st.fullscreen;
    f.set(false);
}

/// Время, дата и день недели — по часам системы (через страницу: так проще
/// всего получить местное время и русские названия месяцев).
const CLOCK_JS: &str = r#"return (() => {
  const d = new Date();
  const time = d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  const date = d.toLocaleDateString("ru-RU", { day: "numeric", month: "long" });
  const year = String(d.getFullYear());
  const wd = d.toLocaleDateString("ru-RU", { weekday: "short" }).replace(".", "");
  return [time, date, year, wd, String(d.getSeconds())];
})()"#;

/// Часы, которые раз в секунду спрашивают время у страницы.
fn use_clock() -> Signal<Vec<String>> {
    let mut now = use_signal(Vec::<String>::new);
    use_future(move || async move {
        loop {
            if let Ok(v) = document::eval(CLOCK_JS).join::<Vec<String>>().await {
                if *now.peek() != v {
                    now.set(v);
                }
            }
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
    });
    now
}

/// Логотип, название и часы с датой и днём недели (сцена и заголовок в стиле «Сцена»).
#[component]
pub fn BrandClock() -> Element {
    let clock = use_clock();
    let c = clock.read().clone();
    rsx! {
        span { class: "stage-logo", dangerous_inner_html: LOGO_SVG }
        span { class: "stage-brand", "Nebula" }
        if c.len() >= 4 {
            div { class: "stage-clock",
                span { class: "stage-time", "{c[0]}" }
                span { class: "stage-date", "{c[1]}" }
            }
            span { class: "stage-chip", "{c[3]}" }
        }
    }
}

/// Плашка текущего трека: обложка, название, «играет». На сцене щелчок —
/// пауза/игра, в основном окне — открыть сцену.
#[component]
pub fn NowPill(#[props(default)] open_stage: bool) -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let playing = matches!(*st.pstate.read(), PlaybackState::Playing | PlaybackState::Buffering);
    let art = now.as_ref().and_then(|t| t.art(ArtSize::Small));
    let Some(t) = now else { return rsx! {} };
    let title = if open_stage { "Открыть сцену (F)" } else if playing { "Пауза" } else { "Играть" };
    rsx! {
        button {
            class: "stage-pill",
            title: "{title}",
            onmousedown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            onclick: move |_| {
                if open_stage {
                    let mut f = st.fullscreen;
                    f.set(true);
                } else {
                    toggle_play(st)
                }
            },
            Cover { src: art, class: "stage-pill-art", loading: "eager" }
            span { class: "stage-pill-title ellipsis", "{t.title}" }
            if playing {
                PlayingBars {}
            } else {
                Icon { name: "play", size: 14 }
            }
        }
    }
}

#[component]
pub fn StageTop() -> Element {
    let st = app_state();
    rsx! {
        header {
            class: "stage-top",
            onmousedown: move |e| {
                if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary) {
                    window().drag();
                }
            },
            ondoubleclick: move |_| toggle_maximized_quiet(),
            div { class: "stage-side", BrandClock {} }
            div { class: "stage-center", NowPill {} }
            div { class: "stage-side end",
                div { class: "stage-switch", onmousedown: move |e| e.stop_propagation(),
                    button { class: "stage-switch-btn", title: "Вернуться в медиатеку (Esc)", onclick: move |_| close_stage(st), "Медиатека" }
                    button { class: "stage-switch-btn active", "Сцена" }
                }
                IconBtn { icon: "search", title: "Найти текст вручную", size: 17, onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); } }
                IconBtn {
                    icon: "fullscreen",
                    title: "Во весь экран (F11)",
                    size: 17,
                    onclick: move |_| {
                        let w = window();
                        let v = w.fullscreen().is_none();
                        w.set_fullscreen(v);
                    },
                }
                span { class: "stage-sep" }
                div { class: "win-btns", onmousedown: move |e| e.stop_propagation(),
                    button { class: "win-btn", title: "Свернуть", onclick: move |_| minimize_app(st), span { class: "wb-min" } }
                    button { class: "win-btn", title: "Развернуть", onclick: move |_| toggle_maximized_quiet(), span { class: "wb-max" } }
                    button { class: "win-btn close", title: "Закрыть", onclick: move |_| close_app(st), Icon { name: "close", size: 16 } }
                }
            }
        }
    }
}

/// Большая обложка текущего трека.
#[component]
pub fn StageCover() -> Element {
    let st = app_state();
    let art = st.now.read().as_ref().and_then(|t| t.art(ArtSize::Large));
    let lv = if st.settings.read().beat_glow { (*st.level.read() * 3.2).min(1.0) } else { 0.0 };
    let playing = matches!(*st.pstate.read(), PlaybackState::Playing | PlaybackState::Buffering);
    rsx! {
        div { class: if playing { "stage-card stage-cover playing" } else { "stage-card stage-cover" }, style: "--beat: {lv:.2};",
            // фон карточки — та же обложка, сильно размытая
            if let Some(a) = art.clone() {
                div { class: "stage-cover-bg", style: "background-image: url('{a}');" }
            }
            // обложка — конверт, из него выезжает пластинка и крутится, пока играет музыка
            // тонарм: опускается на пластинку, когда музыка играет
            svg { class: "stage-arm", view_box: "0 0 60 200", "aria-hidden": "true",
                circle { cx: "30", cy: "22", r: "16", class: "arm-base" }
                circle { cx: "30", cy: "22", r: "6", class: "arm-pivot" }
                path { d: "M30 22 L30 150 L18 176", class: "arm-rod" }
                rect { x: "8", y: "170", width: "18", height: "26", rx: "4", class: "arm-head", transform: "rotate(-24 17 183)" }
            }
            div { class: "stage-sleeve",
                div { class: "stage-vinyl",
                    div { class: "stage-vinyl-label",
                        style: if let Some(a) = art.clone() { format!("background-image: url('{a}');") } else { String::new() },
                    }
                }
                Cover { src: art.clone(), loading: "eager" }
            }
            button {
                class: "stage-cover-btn",
                title: "Во весь экран (F11)",
                onclick: move |_| {
                    let w = window();
                    let v = w.fullscreen().is_none();
                    w.set_fullscreen(v);
                },
                Icon { name: "fullscreen", size: 16 }
            }
        }
    }
}

/// Карточка плеера: название, лайк и звук, кнопки, полоса прогресса.
#[component]
pub fn StagePlayer() -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let order = st.theme.read().layout.controls.clone();
    let liked = now.as_ref().is_some_and(|t| st.library.read().is_favorite(&t.id));
    let (muted, vol) = {
        let s = st.settings.read();
        (s.muted, s.volume)
    };
    let vol_icon = if muted || vol <= 0.001 { "volume_off" } else if vol < 0.5 { "volume_low" } else { "volume" };
    let (title, artist) = match &now {
        Some(t) => (t.title.clone(), t.artist.clone()),
        None => ("Ничего не играет".to_string(), "Выберите трек".to_string()),
    };
    rsx! {
        div { class: "stage-card stage-player",
            div { class: "stage-player-head",
                div { class: "stage-player-meta",
                    div { class: "stage-player-title ellipsis", title: "{title}", "{title}" }
                    div { class: "stage-player-artist ellipsis", "{artist}" }
                }
                if let Some(t) = now.clone() {
                    button {
                        class: if liked { "stage-round active" } else { "stage-round" },
                        title: if liked { "Убрать из избранного" } else { "В избранное" },
                        onclick: move |_| toggle_like(st, &t),
                        Icon { name: if liked { "heart" } else { "heart_outline" }, size: 17 }
                    }
                }
                button { class: "stage-round", title: "Без звука (M)", onclick: move |_| toggle_mute(st), Icon { name: vol_icon, size: 17 } }
            }
            Controls { order }
            StageWave {}
        }
    }
}

/// Прогресс «Сцены» — волна из столбиков: у каждого трека своя форма
/// (по его id), сыгранная часть светлая, под курсором тянется как ползунок.
#[component]
fn StageWave() -> Element {
    const N: usize = 56;
    let st = app_state();
    let dur = *st.duration.read();
    let mut drag = use_signal(|| None::<u64>);
    // позиция приходит 10 раз в секунду — с коротким переходом заливка идёт без рывков
    let pos = drag.read().unwrap_or(*st.position.read());
    let pct = if dur > 0 { (pos as f64 / dur as f64 * 100.0).min(100.0) } else { 0.0 };
    let id = st.now.read().as_ref().map(|t| t.id.clone()).unwrap_or_default();
    let bars = use_memo(use_reactive!(|id| {
        let mut seed = id.bytes().fold(0x9E37_79B9_7F4A_7C15u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3)) | 1;
        let mut prev = 0.5f32;
        (0..N)
            .map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let r = (seed % 1000) as f32 / 1000.0;
                // сглаживаем соседей и приглушаем края — похоже на огибающую песни
                let edge = (std::f32::consts::PI * (i as f32 + 0.5) / N as f32).sin().powf(0.35);
                prev = prev * 0.45 + r * 0.55;
                (0.18 + prev * 0.82 * edge).clamp(0.12, 1.0)
            })
            .collect::<Vec<f32>>()
    }));
    rsx! {
        div { class: "stage-wave-wrap", "data-el": "progress",
            div { class: "stage-wave", style: "--pp: {pct}%;",
                // тусклые столбики и поверх — светлые, открытые до позиции:
                // заливка ползёт плавно, а не перескакивает по столбику
                div { class: "sw-bars base",
                    for (i, h) in bars.read().iter().enumerate() {
                        i { key: "{i}", style: "--h: {h:.3};" }
                    }
                }
                div { class: "sw-bars lit",
                    for (i, h) in bars.read().iter().enumerate() {
                        i { key: "{i}", style: "--h: {h:.3};" }
                    }
                }
                span { class: "sw-head" }
                input {
                    r#type: "range",
                    class: "stage-wave-input",
                    title: "Перемотка",
                    min: "0",
                    max: "{dur.max(1)}",
                    step: "250",
                    value: "{pos}",
                    oninput: move |e: FormEvent| {
                        if let Ok(v) = e.value().parse::<f64>() {
                            drag.set(Some(v as u64));
                        }
                    },
                    onchange: move |e: FormEvent| {
                        if let Ok(v) = e.value().parse::<f64>() {
                            seek(st, v as u64);
                        }
                        let at = *drag.peek();
                        spawn(async move {
                            tokio::time::sleep(Duration::from_millis(500)).await;
                            if *drag.peek() == at {
                                drag.set(None);
                            }
                        });
                    },
                }
            }
            div { class: "stage-wave-times",
                span { "{crate::model::fmt_time(pos)}" }
                span { "−{crate::model::fmt_time(dur.saturating_sub(pos))}" }
            }
        }
    }
}

/// Оформления часов «Сцены»: (id, название).
pub const CLOCK_STYLES: &[(&str, &str)] = &[("ring", "Кольцо"), ("digital", "Цифры"), ("analog", "Стрелки"), ("flip", "Перекидные"), ("bar", "Шкала дня")];

/// Часы: кольцо секунд, время и дата.
#[component]
pub fn StageClock() -> Element {
    let clock = use_clock();
    let c = clock.read().clone();
    // кольцо крутится CSS-анимацией; задержка выравнивает его по секундам системы
    let secs: u32 = c.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut sync = use_signal(|| None::<u32>);
    if sync.peek().is_none() && !c.is_empty() {
        sync.set(Some(secs));
    }
    let delay = sync.read().unwrap_or(0);
    let st = app_state();
    let style = st.settings.read().clock_style.clone();
    let (hh, mm) = c.first().and_then(|t| t.split_once(':')).map(|(h, m)| (h.to_string(), m.to_string())).unwrap_or_default();
    let (h, m): (f32, f32) = (hh.parse().unwrap_or(0.0), mm.parse().unwrap_or(0.0));
    let date = if c.len() >= 4 { format!("{} · {} {}", c[3], c[1], c[2]) } else { String::new() };
    let day_pct = ((h * 60.0 + m) / 1440.0 * 100.0).clamp(0.0, 100.0);
    // перекидные: ключ с цифрой — сменилась цифра, карточка переворачивается
    let flip_cells: Vec<(String, String, bool)> = {
        let mut v = Vec::new();
        for (i, ch) in format!("{hh}{mm}").chars().enumerate() {
            if i == 2 {
                v.push(("sep".to_string(), ":".to_string(), true));
            }
            v.push((format!("{i}-{ch}"), ch.to_string(), false));
        }
        v
    };
    // щелчок по часам — следующее оформление (то же меняется в настройках)
    let next_style = move |_| {
        let mut s = st.settings;
        let cur = s.peek().clock_style.clone();
        let i = CLOCK_STYLES.iter().position(|(id, _)| *id == cur).unwrap_or(0);
        s.write().clock_style = CLOCK_STYLES[(i + 1) % CLOCK_STYLES.len()].0.to_string();
    };
    let title = CLOCK_STYLES.iter().find(|(id, _)| *id == style).map(|(_, n)| *n).unwrap_or("Кольцо");
    rsx! {
        div { class: "stage-card stage-widget stage-clock-card", title: "Оформление: {title} — щёлкните, чтобы сменить", onclick: next_style,
            div { class: "stage-widget-head", span { class: "stage-widget-title", "Часы" } }
            for k in std::iter::once(style.clone()) {
                div { key: "{k}", class: "stage-clock-body cs-{k}",
                    match k.as_str() {
                        "digital" => rsx! {
                            div { class: "stage-clock-text",
                                div { class: "clock-digits",
                                    span { "{hh}" } span { class: "clock-colon", ":" } span { "{mm}" }
                                    span { class: "clock-sec", style: "animation-delay: -{delay}s;" }
                                }
                                div { class: "stage-clock-date", "{date}" }
                            }
                        },
                        "analog" => rsx! {
                            svg { class: "clock-analog", view_box: "0 0 64 64", width: "64", height: "64", "aria-hidden": "true",
                                circle { class: "ca-face", cx: "32", cy: "32", r: "30" }
                                for i in 0..12 {
                                    line { key: "{i}", class: if i % 3 == 0 { "ca-tick big" } else { "ca-tick" }, x1: "32", y1: "4", x2: "32", y2: if i % 3 == 0 { "10" } else { "8" }, transform: "rotate({i * 30} 32 32)" }
                                }
                                line { class: "ca-hour", x1: "32", y1: "32", x2: "32", y2: "17", transform: "rotate({(h % 12.0) * 30.0 + m * 0.5} 32 32)" }
                                line { class: "ca-min", x1: "32", y1: "32", x2: "32", y2: "9", transform: "rotate({m * 6.0} 32 32)" }
                                g { class: "ca-sec-g", style: "animation-delay: -{delay}s;",
                                    line { class: "ca-sec", x1: "32", y1: "37", x2: "32", y2: "7" }
                                }
                                circle { class: "ca-pin", cx: "32", cy: "32", r: "2.4" }
                            }
                            div { class: "stage-clock-text",
                                div { class: "stage-clock-time", "{hh}:{mm}" }
                                div { class: "stage-clock-date", "{date}" }
                            }
                        },
                        "flip" => rsx! {
                            div { class: "stage-clock-text",
                                div { class: "clock-flip",
                                    for (key, ch, sep) in flip_cells.clone() {
                                        span { key: "{key}", class: if sep { "flip-sep" } else { "flip-card" }, "{ch}" }
                                    }
                                }
                                div { class: "stage-clock-date", "{date}" }
                            }
                        },
                        "bar" => rsx! {
                            div { class: "stage-clock-text clock-bar-wrap",
                                div { class: "stage-clock-time", "{hh}:{mm}" }
                                div { class: "clock-bar", div { class: "clock-bar-fill", style: "width: {day_pct}%;" } }
                                div { class: "stage-clock-date", "{date} · прошло {day_pct:.0}% дня" }
                            }
                        },
                        _ => rsx! {
                            svg { class: "stage-ring", view_box: "0 0 44 44", width: "56", height: "56", "aria-hidden": "true",
                                circle { class: "stage-ring-bg", cx: "22", cy: "22", r: "19", fill: "none", stroke_width: "2.5" }
                                circle { class: "stage-ring-fg", cx: "22", cy: "22", r: "19", fill: "none", stroke_width: "2.5", style: "animation-delay: -{delay}s;" }
                            }
                            div { class: "stage-clock-text",
                                div { class: "stage-clock-time", "{hh}:{mm}" }
                                div { class: "stage-clock-date", "{date}" }
                            }
                        },
                    }
                }
            }
        }
    }
}

const VIZ_JS: &str = r#"
while (true) {
  const v = await dioxus.recv();
  const bars = document.querySelectorAll(".stage-viz i");
  for (let i = 0; i < bars.length; i++) bars[i].style.setProperty("--h", (v[i % v.length] || 0).toFixed(3));
}
"#;

/// Столбики спектра (тот же анализатор, что и у визуализатора на главной).
#[component]
fn StageViz() -> Element {
    use std::sync::atomic::Ordering;
    use_hook(|| {
        super::pages::VISUALIZERS.fetch_add(1, Ordering::Relaxed);
        svc().player.set_visualizer(true);
    });
    use_drop(|| {
        if super::pages::VISUALIZERS.fetch_sub(1, Ordering::Relaxed) <= 1 {
            svc().player.set_visualizer(false);
        }
    });
    use_future(|| async move {
        let ev = document::eval(VIZ_JS);
        let mut quiet = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(33)).await;
            let v = svc().player.spectrum();
            if v.iter().all(|x| *x < 0.004) {
                quiet += 1;
                if quiet > 3 {
                    continue;
                }
            } else {
                quiet = 0;
            }
            let vals: Vec<f32> = v.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
            let _ = ev.send(vals);
        }
    });
    let n = crate::audio::SPECTRUM_BANDS;
    rsx! {
        div { class: "stage-viz", "aria-hidden": "true",
            for i in 0..n {
                i { key: "{i}" }
            }
        }
    }
}

/// Мини-эквалайзер: положения полос, пресет и включение.
#[component]
fn StageEq() -> Element {
    let st = app_state();
    let (on, bands, preset, preamp) = {
        let s = st.settings.read();
        (s.eq_enabled, s.eq_bands, s.eq_preset.clone(), s.eq_preamp)
    };
    let preset_name = crate::settings::EQ_PRESETS.iter().find(|(id, _, _)| *id == preset).map(|(_, n, _)| n.to_string()).unwrap_or_else(|| "Свой".into());
    rsx! {
        div { class: "stage-eq",
            div { class: "stage-eq-info",
                span { "10 полос · {preset_name}" }
                button {
                    class: if on { "stage-eq-toggle on" } else { "stage-eq-toggle" },
                    onclick: move |_| { let mut s = st.settings; let v = !s.peek().eq_enabled; s.write().eq_enabled = v; },
                    if on { "Вкл · {preamp:+.0} дБ" } else { "Выкл" }
                }
            }
            div {
                class: if on { "stage-eq-bands" } else { "stage-eq-bands off" },
                title: "Открыть эквалайзер",
                onclick: move |_| super::player_bar::set_eq_open(st, true),
                for i in 0..10usize {
                    {
                        let pct = ((bands[i] + 12.0) / 24.0 * 100.0).clamp(0.0, 100.0);
                        let f = EQ_FREQS[i];
                        let label = if f >= 1000.0 { format!("{}k", f / 1000.0) } else { format!("{f}") };
                        rsx! {
                            div { key: "{i}", class: "stage-eq-band",
                                span { class: "stage-eq-track", span { class: "stage-eq-knob", style: "left: {pct}%;" } }
                                span { class: "stage-eq-freq", "{label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Статистика прослушивания.
#[component]
fn StageStats() -> Element {
    let st = app_state();
    let lib = st.library.read();
    let stats = &lib.listening;
    let total = stats.total_with_imported();
    let top = stats
        .by_track_ms
        .iter()
        .max_by_key(|(_, ms)| **ms)
        .and_then(|(id, _)| lib.history.iter().chain(lib.favorites.iter()).find(|t| &t.id == id))
        .map(|t| format!("{} — {}", t.artist, t.title));
    rsx! {
        div { class: "stage-stats",
            div { class: "stage-stats-row",
                div { class: "stage-stats-big", "{fmt_listening(total)}" }
                div { class: "stage-stats-sub", "всего в Nebula" }
            }
            div { class: "stage-stats-row",
                div { class: "stage-stats-big", "{stats.by_track_ms.len()}" }
                div { class: "stage-stats-sub", "треков в статистике" }
            }
            if let Some(t) = top {
                div { class: "stage-stats-top ellipsis", title: "{t}", "Чаще всего: {t}" }
            }
        }
    }
}

const WIDGETS: [&str; 3] = ["Визуализатор", "Эквалайзер", "Статистика"];

/// Листаемая карточка: визуализатор, эквалайзер, статистика.
#[component]
pub fn StageWidgets() -> Element {
    let mut page = use_signal(|| 0usize);
    // куда листали: следующий виджет въезжает справа, предыдущий — слева
    let mut dir = use_signal(|| 1i8);
    let mut drag_x = use_signal(|| None::<f64>);
    let p = page();
    let n = WIDGETS.len();
    let mut go = move |to: usize, d: i8| {
        dir.set(d);
        page.set(to % n);
    };
    let sw = if dir() >= 0 { "sw-next" } else { "sw-prev" };
    rsx! {
        div { class: "stage-card stage-widget swipe",
            // свайп мышью: потянули влево — следующий, вправо — предыдущий
            onpointerdown: move |e: PointerEvent| drag_x.set(Some(e.client_coordinates().x)),
            onpointerup: move |e: PointerEvent| {
                if let Some(x0) = drag_x.take() {
                    let dx = e.client_coordinates().x - x0;
                    if dx < -40.0 {
                        go(p + 1, 1);
                    } else if dx > 40.0 {
                        go(p + n - 1, -1);
                    }
                }
            },
            onpointerleave: move |_| drag_x.set(None),
            div { class: "stage-widget-head",
                for k in std::iter::once(p) {
                    span { key: "{k}", class: "stage-widget-title {sw}", "{WIDGETS[p]}" }
                }
                div { class: "flex1" }
                button { class: "stage-arrow", title: "Назад", onclick: move |_| go(p + n - 1, -1), Icon { name: "back", size: 14 } }
                button { class: "stage-arrow", title: "Дальше", onclick: move |_| go(p + 1, 1), Icon { name: "forward", size: 14 } }
            }
            div { key: "{p}", class: "stage-widget-body {sw}",
                match p {
                    0 => rsx! { StageViz {} },
                    1 => rsx! { StageEq {} },
                    _ => rsx! { StageStats {} },
                }
            }
            div { class: "stage-dots",
                for i in 0..n {
                    button { key: "{i}", class: if i == p { "stage-dot active" } else { "stage-dot" }, title: "{WIDGETS[i]}",
                        onclick: move |_| go(i, if i >= p { 1 } else { -1 }) }
                }
            }
        }
    }
}
