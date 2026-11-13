//! «Сцена» — полноэкранный режим «сейчас играет»: слева колонка карточек
//! (обложка, плеер, часы и листаемый виджет), справа крупный текст песни.
//! Верхняя полоса — название и часы, плашка текущего трека по центру,
//! переключатель «Музыка / Сцена» по центру и кнопки окна.

use super::actions::*;
use super::chrome::{close_app, minimize_app, toggle_maximized_quiet, ModeSwitch};
use super::icons::{Icon, LOGO_SVG};
use super::player_bar::{Controls, Cover};
use super::settings_page::upd;
use super::state::*;
use super::widgets::IconBtn;
use crate::audio::{PlaybackState, EQ_FREQS};
use crate::model::{file_url, fmt_listening, ArtSize};
use crate::services::svc;
use dioxus::desktop::window;
use dioxus::prelude::*;
use std::time::Duration;

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
            div { class: "stage-center", ModeSwitch {} }
            div { class: "stage-side end",
                IconBtn { icon: "search", title: "Найти текст вручную", size: 17, onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); } }
                IconBtn {
                    icon: "fullscreen",
                    title: "Во весь экран (F11)",
                    size: 17,
                    onclick: move |_| super::chrome::toggle_fullscreen_animated(st),
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
    let (beat_glow, custom_path, last_path) = {
        let s = st.settings.read();
        // старые настройки: фон стоял, а «последнего» ещё не запоминали
        let last = s.stage_cover_bg_last.clone().or_else(|| s.stage_cover_bg.clone());
        (s.beat_glow, s.stage_cover_bg.clone(), last)
    };
    let custom_url = custom_path.as_deref().map(file_url);
    let has_custom = custom_url.is_some();
    // ранее выбранный фон, которым можно снова заменить пластинку
    let last_url = if has_custom { None } else { last_path.as_deref().filter(|p| std::path::Path::new(p).exists()).map(file_url) };
    let lv = if beat_glow { (*st.level.read() * 3.2).min(1.0) } else { 0.0 };
    let playing = matches!(*st.pstate.read(), PlaybackState::Playing | PlaybackState::Buffering);
    rsx! {
        div {
            class: match (playing, has_custom) {
                (true, true) => "stage-card stage-cover playing has-custom-bg",
                (false, true) => "stage-card stage-cover has-custom-bg",
                (true, false) => "stage-card stage-cover playing",
                (false, false) => "stage-card stage-cover",
            },
            style: "--beat: {lv:.2};",
            if let Some(bg) = custom_url {
                img {
                    class: "stage-cover-custom",
                    src: "{bg}",
                    alt: "",
                    draggable: "false",
                }
            } else {
                // фон карточки — та же обложка, сильно размытая
                if let Some(a) = art.clone() {
                    div { class: "stage-cover-bg", style: "background-image: url('{a}');" }
                }
                // обложка — конверт, из него выезжает пластинка и крутится, пока играет музыка
                // тонарм: опускается на пластинку, когда музыка играет
                StageArm {}
                div { class: "stage-sleeve",
                    div { class: "stage-vinyl",
                        div { class: "stage-vinyl-label",
                            style: if let Some(a) = art.clone() { format!("background-image: url('{a}');") } else { String::new() },
                        }
                    }
                    Cover { src: art.clone(), loading: "eager" }
                }
            }
            // при наведении: на пластинке — поставить прошлый фон (превью) или выбрать
            // новый; на своём фоне — вернуть пластинку или выбрать другой
            div { class: "stage-cover-actions",
                if has_custom {
                    button { class: "stage-cover-bg-btn", title: "Вернуть виниловую пластинку",
                        onclick: move |_| upd(st, |s| {
                            if s.stage_cover_bg.is_some() {
                                s.stage_cover_bg_last = s.stage_cover_bg.take();
                            }
                        }),
                        Icon { name: "album", size: 17 }
                    }
                } else if let Some(u) = last_url.clone() {
                    button { class: "stage-cover-bg-btn stage-cover-last", title: "Поставить прошлый фон",
                        onclick: move |_| upd(st, |s| s.stage_cover_bg = s.stage_cover_bg_last.clone().or_else(|| s.stage_cover_bg.clone())),
                        img { src: "{u}", alt: "", draggable: "false" }
                    }
                }
                button {
                    class: "stage-cover-bg-btn",
                    title: if has_custom { "Выбрать другое фото или GIF" } else if last_url.is_some() { "Выбрать новое фото или GIF" } else { "Поменять фон (фото или GIF)" },
                    onclick: move |_| pick_cover_bg(st),
                    Icon { name: if has_custom || last_url.is_some() { "add_photo" } else { "image" }, size: 16 }
                }
            }
        }
    }
}

/// Выбрать своё фото или GIF вместо пластинки — запоминается как «последний фон».
fn pick_cover_bg(st: AppState) {
    spawn(async move {
        let picked = rfd::AsyncFileDialog::new()
            .set_title("Выберите изображение или GIF")
            .add_filter(
                "Изображения и GIF",
                &["png", "jpg", "jpeg", "jfif", "pjpeg", "pjp", "gif", "webp", "avif", "apng", "bmp", "ico", "tif", "tiff", "heic", "heif", "svg"],
            )
            .add_filter("Все файлы", &["*"])
            .pick_file()
            .await;
        if let Some(h) = picked {
            let p = h.path().to_string_lossy().into_owned();
            upd(st, |s| {
                s.stage_cover_bg = Some(p.clone());
                s.stage_cover_bg_last = Some(p);
            });
        }
    });
}

/// Тонарм проигрывателя: основание с подшипником, противовес, металлическая
/// трубка, головка с картриджем, иглой и подъёмником. Металл — градиенты,
/// блики и тени, чтобы не выглядел плоским.
#[component]
fn StageArm() -> Element {
    rsx! {
        svg { class: "stage-arm", view_box: "0 0 60 200", "aria-hidden": "true",
            defs {
                radialGradient { id: "arm-plate", cx: "40%", cy: "35%", r: "70%",
                    stop { offset: "0", stop_color: "#3a3a3d" }
                    stop { offset: "0.7", stop_color: "#1b1b1d" }
                    stop { offset: "1", stop_color: "#0e0e10" }
                }
                radialGradient { id: "arm-cap", cx: "38%", cy: "32%", r: "75%",
                    stop { offset: "0", stop_color: "#ffffff" }
                    stop { offset: "0.45", stop_color: "#c9c9cc" }
                    stop { offset: "1", stop_color: "#6c6c70" }
                }
                linearGradient { id: "arm-tube", x1: "0", y1: "0", x2: "1", y2: "0",
                    stop { offset: "0", stop_color: "#7d7d82" }
                    stop { offset: "0.35", stop_color: "#f4f4f6" }
                    stop { offset: "0.6", stop_color: "#bdbdc2" }
                    stop { offset: "1", stop_color: "#5d5d62" }
                }
                linearGradient { id: "arm-weight", x1: "0", y1: "0", x2: "1", y2: "0",
                    stop { offset: "0", stop_color: "#2a2a2d" }
                    stop { offset: "0.4", stop_color: "#5a5a5f" }
                    stop { offset: "0.55", stop_color: "#3a3a3e" }
                    stop { offset: "1", stop_color: "#151517" }
                }
                linearGradient { id: "arm-shell", x1: "0", y1: "0", x2: "1", y2: "1",
                    stop { offset: "0", stop_color: "#3b3b3f" }
                    stop { offset: "1", stop_color: "#141416" }
                }
            }
            // основание: плита с кольцом и винтами
            circle { cx: "30", cy: "22", r: "18", fill: "url(#arm-plate)", class: "arm-base" }
            circle { cx: "30", cy: "22", r: "13.5", fill: "none", stroke: "rgba(255,255,255,.09)", stroke_width: "1" }
            for (x, y) in [(17.5f32, 12.0f32), (42.5, 12.0), (17.5, 32.0), (42.5, 32.0)] {
                circle { key: "{x}-{y}", cx: "{x}", cy: "{y}", r: "1.1", fill: "#77777c" }
            }
            // противовес над шарниром (рифлёный цилиндр)
            rect { x: "23", y: "2", width: "14", height: "11", rx: "2.5", fill: "url(#arm-weight)", stroke: "rgba(0,0,0,.5)", stroke_width: ".6" }
            for k in 0..4 {
                line { key: "{k}", x1: "{25.5 + k as f32 * 3.0}", y1: "3.2", x2: "{25.5 + k as f32 * 3.0}", y2: "11.8", stroke: "rgba(255,255,255,.12)", stroke_width: ".6" }
            }
            rect { x: "28.6", y: "11", width: "2.8", height: "6", fill: "url(#arm-tube)" }
            // трубка: мягкий изгиб к головке, с бликом вдоль
            path { d: "M30 22 L30 132 Q30 146 22.5 160", fill: "none", stroke: "rgba(0,0,0,.45)", stroke_width: "6.2", stroke_linecap: "round", class: "arm-rod-shadow" }
            path { d: "M30 22 L30 132 Q30 146 22.5 160", fill: "none", stroke: "url(#arm-tube)", stroke_width: "4.6", stroke_linecap: "round", class: "arm-rod" }
            path { d: "M29.2 26 L29.2 131", fill: "none", stroke: "rgba(255,255,255,.75)", stroke_width: ".8", stroke_linecap: "round" }
            // шарнир (подшипник) поверх трубки
            circle { cx: "30", cy: "22", r: "7.5", fill: "#0f0f11", stroke: "rgba(255,255,255,.18)", stroke_width: ".8" }
            circle { cx: "30", cy: "22", r: "5.2", fill: "url(#arm-cap)", class: "arm-pivot" }
            circle { cx: "28.4", cy: "20.4", r: "1.4", fill: "rgba(255,255,255,.85)" }
            // головка: корпус, подъёмник, картридж с акцентной вставкой и игла
            g { transform: "rotate(-26 21 172)",
                rect { x: "18.6", y: "157", width: "6", height: "5", rx: "1.2", fill: "#9c9ca1" }
                rect { x: "11", y: "161", width: "20", height: "24", rx: "3.5", fill: "url(#arm-shell)", stroke: "rgba(255,255,255,.16)", stroke_width: ".8", class: "arm-head" }
                path { d: "M31 166 q7 -1 8 -7", fill: "none", stroke: "#bdbdc2", stroke_width: "1.6", stroke_linecap: "round" }
                rect { x: "13.5", y: "170", width: "15", height: "12", rx: "1.8", fill: "#0b0b0c" }
                rect { x: "13.5", y: "170", width: "15", height: "2.4", rx: "1", class: "arm-accent" }
                line { x1: "15.5", y1: "175", x2: "26.5", y2: "175", stroke: "rgba(255,255,255,.08)", stroke_width: ".6" }
                line { x1: "15.5", y1: "178", x2: "26.5", y2: "178", stroke: "rgba(255,255,255,.08)", stroke_width: ".6" }
                path { d: "M21 182 L21 187.5", stroke: "#d8d8dc", stroke_width: ".9", stroke_linecap: "round" }
                circle { cx: "21", cy: "188", r: ".9", fill: "#fff" }
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
    let pstate = *st.pstate.read();
    let playing_u8 = u8::from(pstate == PlaybackState::Playing);
    rsx! {
        div { class: "stage-wave-wrap", "data-el": "progress",
            div {
                class: "stage-wave",
                style: "--pp: {pct}%;",
                "data-pos": "{pos}",
                "data-dur": "{dur}",
                "data-play": "{playing_u8}",
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
const cur = new Float32Array(48);
const tgt = new Float32Array(48);
let active = false, last = 0;
function tick(now) {
  const dt = last ? Math.min(0.05, Math.max(0.001, (now - last) / 1000)) : 1 / 60;
  last = now;
  const k = 1 - Math.pow(0.42, dt * 60);
  const bars = document.querySelectorAll(".stage-viz i");
  let energy = 0;
  for (let i = 0; i < bars.length; i++) {
    const t = tgt[i % 24] || 0;
    const c = cur[i] + (t - cur[i]) * k;
    cur[i] = c;
    energy += Math.abs(c) + Math.abs(t);
    bars[i].style.setProperty("--h", c.toFixed(3));
  }
  if (energy > 0.002) {
    requestAnimationFrame(tick);
  } else {
    active = false;
    last = 0;
  }
}
while (true) {
  const v = await dioxus.recv();
  const n = Math.max(1, v.length);
  for (let i = 0; i < 24; i++) tgt[i] = v[i % n] || 0;
  if (!active) {
    active = true;
    last = 0;
    requestAnimationFrame(tick);
  }
}
"#;

/// Столбики спектра (тот же анализатор, что и у визуализатора на главной),
/// подстроенные под частоту обновления монитора.
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
            tokio::time::sleep(super::display::frame_interval(super::display::app_hz(), super::display::low_power())).await;
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
