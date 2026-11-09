//! Панель плеера: обложка и название, кнопки управления (порядок задаётся темой),
//! полоса прогресса с буфером, громкость до 400% и всплывающий эквалайзер.

use super::actions::*;
use super::icons::Icon;
use super::state::*;
use super::widgets::{IconBtn, Slider, Toggle};
use crate::audio::{PlaybackState, EQ_FREQS};
use crate::model::{fmt_time, ArtSize};
use crate::settings::{RepeatMode, EQ_PRESETS};
use crate::theme::{Control, CoverShape, Extra, PlayerStyle};
use dioxus::prelude::*;

#[component]
pub fn Cover(src: Option<String>, #[props(default)] class: String, #[props(default)] spinning: bool, #[props(default = "lazy".to_string())] loading: String) -> Element {
    rsx! {
        div { class: "cover {class}", "data-el": "cover",
            class: if spinning { "spin" },
            match src {
                Some(s) => rsx! { img { src: "{s}", alt: "", loading: "{loading}", decoding: "async", draggable: false } },
                None => rsx! { div { class: "cover-ph", Icon { name: "music", size: 22 } } },
            }
        }
    }
}

#[component]
pub fn PlayerBar() -> Element {
    let st = app_state();
    let layout = st.theme.read().layout.clone();
    let style = match layout.player_style {
        PlayerStyle::Classic => "ps-classic",
        PlayerStyle::Centered => "ps-centered",
        PlayerStyle::Minimal => "ps-minimal",
    };
    // пока идёт переход в островок, плеер доигрывает уход вниз
    let leaving = *st.player_switching.read();
    rsx! {
        footer { class: if leaving { "player {style} leaving" } else { "player {style}" }, "data-el": "player",
            NowPlayingInfo {}
            div { class: "p-center",
                Controls { order: layout.controls.clone() }
                if layout.player_style != PlayerStyle::Minimal {
                    Progress {}
                }
            }
            Extras { order: layout.extras.clone() }
        }
    }
}

/// Островок — нижний плеер, свёрнутый в заголовок окна (слева от панели
/// разделов). Показывается только когда плеер свёрнут. Сам островок —
/// компактная капсула (обложка, название, пульс, пауза); при наведении под ним
/// выпадает карточка с перемоткой, переключением треков и кнопкой «развернуть».
#[component]
pub fn Island() -> Element {
    let mut st = app_state();
    let track = st.now.read().clone();
    let playing = matches!(*st.pstate.read(), PlaybackState::Playing | PlaybackState::Buffering);
    let mut drag = use_signal(|| None::<u64>);
    let pos = drag.read().unwrap_or((*st.position.read() / 250) * 250);
    let dur = *st.duration.read();
    let pct = if dur > 0 { (pos as f64 / dur as f64 * 100.0).min(100.0) } else { 0.0 };
    let art = track.as_ref().and_then(|t| t.art(ArtSize::Medium));
    let bg_style = art.as_deref().map(|a| format!("--di-art: url('{a}');")).unwrap_or_default();
    let (title, artist) = match &track {
        Some(t) => (t.title.clone(), t.artist.clone()),
        None => ("Ничего не играет".to_string(), "Выберите трек".to_string()),
    };
    let has_track = track.is_some();
    let leaving = *st.player_switching.read();
    rsx! {
        div {
            class: if leaving { "island leaving" } else { "island" },
            "data-el": "dynamic_island",
            // островок живёт в заголовке окна: не даём начать перетаскивание окна
            onmousedown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| {
                e.stop_propagation();
                set_player_collapsed(st, false);
            },
            div { class: "island-pill",
                Cover { src: art.clone(), class: "di-cover", loading: "eager" }
                div { class: "di-meta",
                    div { class: "di-title ellipsis", "{title}" }
                    div { class: "di-artist ellipsis", "{artist}" }
                }
                span { class: if playing { "di-bars on" } else { "di-bars" },
                    span {} span {} span {}
                }
                button {
                    class: "di-play small",
                    title: if playing { "Пауза" } else { "Играть" },
                    disabled: !has_track,
                    ondoubleclick: move |e| e.stop_propagation(),
                    onclick: move |_| toggle_play(st),
                    Icon { name: if playing { "pause" } else { "play" }, size: 16 }
                }
            }
            div { class: "island-card", style: "{bg_style}",
                Cover { src: art, class: "di-cover-large", loading: "eager" }
                div { class: "di-expanded-main",
                    div { class: "di-meta",
                        div { class: "di-title ellipsis", "{title}" }
                        div { class: "di-artist ellipsis", "{artist}" }
                    }
                    div { class: "di-controls",
                        IconBtn { icon: "prev", title: "Предыдущий", size: 18, onclick: move |_| prev(st) }
                        button { class: "di-play", disabled: !has_track, onclick: move |_| toggle_play(st), Icon { name: if playing { "pause" } else { "play" }, size: 18 } }
                        IconBtn { icon: "next", title: "Следующий", size: 18, onclick: move |_| next(st, true) }
                    }
                    div { class: "di-progress",
                        span { class: "time", "{fmt_time(pos)}" }
                        input {
                            r#type: "range",
                            min: "0",
                            max: "{dur.max(1)}",
                            step: "250",
                            value: "{pos}",
                            disabled: !has_track,
                            style: "--di-p: {pct}%;",
                            oninput: move |e: FormEvent| {
                                if let Ok(v) = e.value().parse::<f64>() { drag.set(Some(v as u64)); }
                            },
                            onchange: move |e: FormEvent| {
                                if let Ok(v) = e.value().parse::<f64>() { seek(st, v as u64); }
                                hold_drag(drag);
                            },
                        }
                        span { class: "time", "{fmt_time(dur)}" }
                    }
                }
                div { class: "island-side",
                    IconBtn { icon: "to_bottom", title: "Развернуть плеер внизу", size: 18, onclick: move |_| set_player_collapsed(st, false) }
                    if let Some(t) = track.clone() {
                        IconBtn { icon: "more", title: "Ещё", size: 18,
                            onclick: move |e: MouseEvent| {
                                let p = e.client_coordinates();
                                st.menu.set(Some(MenuState { x: p.x, y: p.y + 12.0, track: t.clone(), playlist: None, queue_index: None }));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NowPlayingInfo() -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let shape = st.theme.read().layout.cover_shape;
    let playing = matches!(*st.pstate.read(), PlaybackState::Playing);
    let preview = *st.preview.read();
    let liked = now.as_ref().map(|t| st.library.read().is_favorite(&t.id)).unwrap_or(false);
    let shape_class = match shape {
        CoverShape::Square => "shape-square",
        CoverShape::Rounded => "shape-rounded",
        CoverShape::Circle => "shape-circle",
        CoverShape::Vinyl => "shape-vinyl",
    };
    let Some(t) = now else {
        return rsx! {
            div { class: "p-left", "data-el": "player_info",
                div { class: "p-cover-wrap {shape_class}", "data-el": "player_cover", Cover { src: None } }
                div { class: "p-meta",
                    div { class: "p-title muted", "Ничего не играет" }
                    div { class: "p-artist", "Выберите трек, чтобы начать полёт" }
                }
            }
        };
    };
    let art = t.art(ArtSize::Medium);
    let t2 = t.clone();
    let t3 = t.clone();
    let uploader = t.uploader_urn.clone().map(|urn| crate::model::UserInfo { urn, username: t.uploader.clone(), ..Default::default() });
    rsx! {
        div { class: "p-left", "data-el": "player_info",
            div {
                class: "p-cover-wrap {shape_class}",
                "data-el": "player_cover",
                title: "Текст на весь экран",
                onclick: move |_| {
                    let mut f = st.fullscreen;
                    f.set(true);
                },
                BeatGlow {}
                Cover { src: art, spinning: shape == CoverShape::Vinyl && playing }
            }
            div { class: "p-meta",
                div { class: "p-title", title: "{t.title}",
                    span { class: "marquee", "{t.title}" }
                    if preview {
                        span { class: "badge", "превью" }
                    }
                }
                div {
                    class: "p-artist link",
                    onclick: move |_| {
                        if let Some(u) = uploader.clone() {
                            navigate(st, Route::User(u));
                        }
                    },
                    "{t.artist}"
                }
            }
            button {
                class: if liked { "icon-btn like on" } else { "icon-btn like" },
                "data-el": "button",
                title: if liked { "Убрать из избранного" } else { "В избранное" },
                onclick: move |_| toggle_like(st, &t2),
                Icon { name: if liked { "heart" } else { "heart_outline" }, size: 19 }
            }
            IconBtn { icon: "more", title: "Ещё", size: 18,
                onclick: move |e: MouseEvent| {
                    let p = e.client_coordinates();
                    let mut m = st.menu;
                    m.set(Some(MenuState { x: p.x, y: p.y - 260.0, track: t3.clone(), playlist: None, queue_index: None }));
                },
            }
        }
    }
}

/// Мягкое свечение за обложкой, пульсирующее в такт басу.
#[component]
fn BeatGlow() -> Element {
    let st = app_state();
    let enabled = st.settings.read().beat_glow && !st.settings.read().low_power;
    if !enabled {
        return rsx! {};
    }
    let lv = (*st.level.read() * 3.2).min(1.0);
    rsx! {
        span { class: "beat-glow", style: "--beat: {lv:.2};" }
    }
}

#[component]
pub fn Controls(order: Vec<Control>) -> Element {
    let st = app_state();
    let pstate = *st.pstate.read();
    let (shuffle, repeat) = {
        let s = st.settings.read();
        (s.shuffle, s.repeat)
    };
    let playing = matches!(pstate, PlaybackState::Playing | PlaybackState::Buffering);
    let busy = matches!(pstate, PlaybackState::Loading | PlaybackState::Buffering);
    rsx! {
        div { class: "controls", "data-el": "controls",
            for c in order {
                match c {
                    Control::Shuffle => rsx! {
                        IconBtn { key: "{c:?}", icon: "shuffle", title: "Перемешать (S)", active: shuffle, size: 18, onclick: move |_| toggle_shuffle(st) }
                    },
                    Control::Prev => rsx! {
                        IconBtn { key: "{c:?}", icon: "prev", title: "Предыдущий (Ctrl+←)", size: 22, onclick: move |_| prev(st) }
                    },
                    Control::Play => rsx! {
                        button {
                            key: "{c:?}",
                            class: if playing { "play-btn playing" } else { "play-btn" },
                            "data-el": "play_button",
                            title: "Пробел",
                            onclick: move |_| toggle_play(st),
                            if busy {
                                span { class: "play-busy" }
                            }
                            Icon { name: if playing { "pause" } else { "play" }, size: 24 }
                        }
                    },
                    Control::Next => rsx! {
                        IconBtn { key: "{c:?}", icon: "next", title: "Следующий (Ctrl+→)", size: 22, onclick: move |_| next(st, true) }
                    },
                    Control::Repeat => rsx! {
                        IconBtn {
                            key: "{c:?}",
                            icon: if repeat == RepeatMode::One { "repeat_one" } else { "repeat" },
                            title: "Повтор (R)",
                            active: repeat != RepeatMode::Off,
                            size: 18,
                            onclick: move |_| cycle_repeat(st),
                        }
                    },
                }
            }
        }
    }
}

#[component]
pub fn Progress() -> Element {
    let st = app_state();
    // перерисовываемся 4 раза в секунду, CSS-переход делает движение плавным
    let quarter = use_memo(move || *st.position.read() / 250);
    let dur = *st.duration.read();
    let buffered = *st.buffered.read();
    let mut drag = use_signal(|| None::<u64>);
    let pos = drag.read().unwrap_or(quarter() * 250);
    let pct = if dur > 0 { (pos as f64 / dur as f64 * 100.0).min(100.0) } else { 0.0 };
    let buf_pct = (buffered * 100.0).clamp(0.0, 100.0);
    let pstate = *st.pstate.read();
    rsx! {
        div { class: "progress-wrap", "data-el": "progress",
            span { class: "time", "{fmt_time(pos)}" }
            div { class: if pstate == PlaybackState::Buffering { "progress buffering" } else { "progress" }, style: "--pp: {pct}%;",
                div { class: "pr-buf", style: "width: {buf_pct}%;" }
                div { class: "pr-fill", style: "width: {pct}%;" }
                input {
                    r#type: "range",
                    class: "pr-input",
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
                        hold_drag(drag);
                    },
                }
            }
            span { class: "time", "{fmt_time(dur)}" }
        }
    }
}

#[component]
fn Extras(order: Vec<Extra>) -> Element {
    let st = app_state();
    let (lyrics_open, queue_open) = {
        let s = st.settings.read();
        (s.lyrics_open, s.queue_open)
    };
    let eq_open = *st.eq_open.read();
    let fs = *st.fullscreen.read();
    rsx! {
        div { class: "p-right", "data-el": "extras",
            for x in order {
                match x {
                    Extra::Like => rsx! {},
                    Extra::Lyrics => rsx! {
                        IconBtn { key: "{x:?}", icon: "lyrics", title: "Текст сбоку (L)", active: lyrics_open, size: 19,
                            onclick: move |_| crate::ui::actions::toggle_lyrics(st) }
                    },
                    Extra::Fullscreen => rsx! {
                        IconBtn { key: "{x:?}", icon: "fullscreen", title: "Текст на весь экран (F)", active: fs, size: 19,
                            onclick: move |_| { let mut f = st.fullscreen; let v = !*f.peek(); f.set(v); } }
                    },
                    Extra::Queue => rsx! {
                        IconBtn { key: "{x:?}", icon: "queue", title: "Очередь (Q)", active: queue_open, size: 19,
                            onclick: move |_| { let mut s = st.settings; let v = !s.peek().queue_open; s.write().queue_open = v; } }
                    },
                    Extra::Eq => rsx! {
                        IconBtn { key: "{x:?}", icon: "eq", title: "Эквалайзер", active: eq_open, size: 19,
                            onclick: move |_| { let mut e = st.eq_open; let v = !*e.peek(); e.set(v); } }
                    },
                    Extra::Volume => rsx! { Volume { key: "{x:?}" } },
                }
            }
            span { class: "p-sep" }
            IconBtn { icon: "to_top", title: "Свернуть плеер в островок", size: 19, class: "p-collapse",
                onclick: move |_| set_player_collapsed(st, true) }
        }
    }
}

#[component]
pub fn Volume() -> Element {
    let st = app_state();
    let (vol, max, muted) = {
        let s = st.settings.read();
        (s.volume, s.max_volume, s.muted)
    };
    let pct = (vol * 100.0).round() as i32;
    let fill = (vol / max * 100.0).clamp(0.0, 100.0);
    let unity = (100.0 / max).clamp(0.0, 100.0);
    let icon = if muted || vol <= 0.001 {
        "volume_off"
    } else if vol > 1.0 {
        "volume_boost"
    } else if vol < 0.5 {
        "volume_low"
    } else {
        "volume"
    };
    let boosted = vol > 1.0 && !muted;
    rsx! {
        div {
            class: if boosted { "volume boosted" } else { "volume" },
            "data-el": "volume",
            onwheel: move |e: WheelEvent| {
                let dy = e.delta().strip_units().y;
                let step = st.settings.peek().volume_step;
                let cur = st.settings.peek().volume;
                set_volume(st, cur + if dy < 0.0 { step } else { -step });
            },
            IconBtn { icon: icon, title: "Без звука (M)", size: 19, onclick: move |_| toggle_mute(st) }
            div { class: "vol-track",
                div { class: "vol-fill", style: "width: {fill}%;" }
                if max > 1.0 {
                    span { class: "vol-unity", style: "left: {unity}%;", title: "100%" }
                }
                input {
                    r#type: "range",
                    class: "vol-input",
                    min: "0",
                    max: "{max}",
                    step: "0.01",
                    value: "{vol}",
                    oninput: move |e: FormEvent| {
                        if let Ok(v) = e.value().parse::<f32>() {
                            // «прилипание» к 100%
                            let v = if (v - 1.0).abs() < 0.025 { 1.0 } else { v };
                            set_volume(st, v);
                        }
                    },
                }
            }
            span { class: "vol-pct", "{pct}%" }
        }
    }
}

#[component]
pub fn EqPanel() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let bands = s.eq_bands;
    rsx! {
        div { class: "eq-backdrop", onclick: move |_| { let mut e = st.eq_open; e.set(false); } }
        div { class: "eq-panel glass", "data-el": "eq_panel", onclick: move |e| e.stop_propagation(),
            div { class: "eq-head",
                h3 { Icon { name: "eq", size: 18 } "Эквалайзер" }
                Toggle { value: s.eq_enabled, onchange: move |v| { let mut ss = st.settings; ss.write().eq_enabled = v; } }
                div { class: "flex1" }
                IconBtn { icon: "close", size: 18, title: "Закрыть", onclick: move |_| { let mut e = st.eq_open; e.set(false); } }
            }
            div { class: "eq-presets",
                for (id , name , vals) in EQ_PRESETS.iter() {
                    button {
                        key: "{id}",
                        class: if s.eq_preset == *id { "chip active" } else { "chip" },
                        "data-el": "chip",
                        onclick: move |_| {
                            let mut ss = st.settings;
                            let mut w = ss.write();
                            w.eq_bands = *vals;
                            w.eq_preset = id.to_string();
                            w.eq_enabled = true;
                        },
                        "{name}"
                    }
                }
            }
            div { class: if s.eq_enabled { "eq-bands" } else { "eq-bands off" },
                for i in 0..10usize {
                    div { key: "{i}", class: "eq-band",
                        span { class: "eq-db", "{bands[i]:+.0}" }
                        input {
                            r#type: "range",
                            class: "eq-input",
                            min: "-12",
                            max: "12",
                            step: "0.5",
                            value: "{bands[i]}",
                            oninput: move |e: FormEvent| {
                                if let Ok(v) = e.value().parse::<f32>() {
                                    let mut ss = st.settings;
                                    let mut w = ss.write();
                                    w.eq_bands[i] = v;
                                    w.eq_preset = "custom".into();
                                    w.eq_enabled = true;
                                }
                            },
                        }
                        span { class: "eq-freq",
                            if EQ_FREQS[i] >= 1000.0 { "{EQ_FREQS[i] / 1000.0}k" } else { "{EQ_FREQS[i]}" }
                        }
                    }
                }
            }
            div { class: "eq-extra",
                div { class: "eq-row",
                    span { "Предусиление" }
                    Slider { value: s.eq_preamp, min: -12.0, max: 6.0, step: 0.5, unit: " дБ", decimals: 1,
                        oninput: move |v| { let mut ss = st.settings; ss.write().eq_preamp = v; } }
                }
                div { class: "eq-row",
                    span { "Баланс" }
                    Slider { value: s.balance, min: -1.0, max: 1.0, step: 0.05, decimals: 2, mark: Some(0.0),
                        oninput: move |v: f32| { let mut ss = st.settings; ss.write().balance = if v.abs() < 0.04 { 0.0 } else { v }; } }
                }
                div { class: "eq-row",
                    span { "Лимитер" }
                    Toggle { value: s.limiter, onchange: move |v| { let mut ss = st.settings; ss.write().limiter = v; } }
                    span { "Моно" }
                    Toggle { value: s.mono, onchange: move |v| { let mut ss = st.settings; ss.write().mono = v; } }
                }
            }
        }
    }
}

/// После отпускания ползунка ещё полсекунды показываем выбранную позицию:
/// плеер успевает перемотаться, и заливка не «отпрыгивает» назад.
fn hold_drag(mut drag: Signal<Option<u64>>) {
    let at = *drag.peek();
    spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        if *drag.peek() == at {
            drag.set(None);
        }
    });
}
