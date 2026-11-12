//! «Сцена» — полноэкранный режим «сейчас играет»: слева колонка карточек
//! (обложка, плеер, часы и листаемый виджет), справа крупный текст песни.
//! Верхняя полоса — название и часы, плашка текущего трека по центру,
//! переключатель «Медиатека / Сцена» и кнопки окна.

use super::actions::*;
use super::chrome::{close_app, minimize_app, toggle_maximized_quiet};
use super::icons::{Icon, LOGO_SVG};
use super::player_bar::{Controls, Cover, Progress};
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
    rsx! {
        div { class: "stage-card stage-cover", style: "--beat: {lv:.2};",
            Cover { src: art, loading: "eager" }
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
            Progress {}
        }
    }
}

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
    rsx! {
        div { class: "stage-card stage-widget",
            div { class: "stage-widget-head", span { class: "stage-widget-title", "Часы" } }
            div { class: "stage-clock-body",
                svg { class: "stage-ring", view_box: "0 0 44 44", width: "56", height: "56", "aria-hidden": "true",
                    circle { class: "stage-ring-bg", cx: "22", cy: "22", r: "19", fill: "none", stroke_width: "2.5" }
                    circle { class: "stage-ring-fg", cx: "22", cy: "22", r: "19", fill: "none", stroke_width: "2.5", style: "animation-delay: -{delay}s;" }
                }
                if c.len() >= 4 {
                    div { class: "stage-clock-text",
                        div { class: "stage-clock-time", "{c[0]}" }
                        div { class: "stage-clock-date", "{c[3]} · {c[1]} {c[2]}" }
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
                onclick: move |_| { let mut e = st.eq_open; e.set(true); },
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
    let p = page();
    let n = WIDGETS.len();
    rsx! {
        div { class: "stage-card stage-widget",
            div { class: "stage-widget-head",
                span { class: "stage-widget-title", "{WIDGETS[p]}" }
                div { class: "flex1" }
                button { class: "stage-arrow", title: "Назад", onclick: move |_| page.set((p + n - 1) % n), Icon { name: "back", size: 14 } }
                button { class: "stage-arrow", title: "Дальше", onclick: move |_| page.set((p + 1) % n), Icon { name: "forward", size: 14 } }
            }
            div { key: "{p}", class: "stage-widget-body",
                match p {
                    0 => rsx! { StageViz {} },
                    1 => rsx! { StageEq {} },
                    _ => rsx! { StageStats {} },
                }
            }
            div { class: "stage-dots",
                for i in 0..n {
                    button { key: "{i}", class: if i == p { "stage-dot active" } else { "stage-dot" }, title: "{WIDGETS[i]}", onclick: move |_| page.set(i) }
                }
            }
        }
    }
}
