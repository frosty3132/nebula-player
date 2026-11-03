//! Корневой компонент Nebula: состояние, раскладка, глобальные обработчики
//! (клавиатура, медиаклавиши, фокус окна), фоновые задачи (статус плеера,
//! события, предзагрузка) и автосохранение настроек/тем/библиотеки.

mod actions;
mod background;
mod chrome;
mod icons;
mod lyrics_view;
mod overlays;
mod pages;
mod player_bar;
mod settings_page;
mod state;
mod theme_editor;
mod tracks;
mod widgets;

use crate::audio::{PlaybackState, PlayerEvent};
use crate::library::{LibraryData, LocalIndex};
use crate::services::svc;
use crate::settings::{Settings, WindowEffect};
use crate::theme::{self, PlayerPos, Side, SidebarPos, ELEMENTS};
use actions::*;
use background::Background;
use chrome::{ResizeHandles, Sidebar, TitleBar};
use dioxus::desktop::{use_asset_handler, use_global_shortcut, use_wry_event_handler, window, HotKeyState};
use dioxus::prelude::*;
use lyrics_view::{FullscreenLyrics, LyricsPanel};
use overlays::{ContextMenu, EditHint, ModalHost, QueuePanel, Toasts};
use pages::*;
use player_bar::{EqPanel, PlayerBar};
use serde::Deserialize;
use settings_page::SettingsPage;
pub use state::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use theme_editor::Inspector;

const BOOT_JS: &str = r#"
const send = (m) => { try { dioxus.send(m); } catch (e) {} };
const NAMES = __NAMES__;
const KEYS = new Set(["Space","ArrowLeft","ArrowRight","ArrowUp","ArrowDown","KeyL","KeyF","KeyQ","KeyM","KeyS","KeyR","Escape","Slash","F11"]);
const CTRL = new Set(["KeyF","KeyE","Comma","ArrowLeft","ArrowRight"]);
window.addEventListener("keydown", (e) => {
  const t = e.target;
  const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
  if (typing && e.code !== "Escape") return;
  if (e.ctrlKey ? !CTRL.has(e.code) : !KEYS.has(e.code)) return;
  if (e.altKey || e.metaKey) return;
  e.preventDefault();
  send({ k: e.code, ctrl: e.ctrlKey, shift: e.shiftKey });
}, true);
window.addEventListener("blur", () => send({ k: "__blur" }));
window.addEventListener("focus", () => send({ k: "__focus" }));
document.addEventListener("visibilitychange", () => {
  send({ k: document.hidden ? "__hidden" : "__visible" });
  document.querySelectorAll("video.bg-video").forEach(v => { try { document.hidden ? v.pause() : v.play(); } catch (e) {} });
});
const editing = () => { const r = document.querySelector(".app"); return r && r.classList.contains("edit-mode"); };
document.addEventListener("click", (e) => {
  if (!editing()) return;
  if (e.target.closest(".inspector, .edit-hint, .win-btns, .tb-edit, .modal-backdrop")) return;
  const el = e.target.closest("[data-el]");
  if (el) { e.preventDefault(); e.stopPropagation(); send({ k: "__select", v: el.getAttribute("data-el") }); }
}, true);
let hovered = null;
document.addEventListener("mouseover", (e) => {
  if (!editing()) { if (hovered) { hovered.classList.remove("el-hover"); hovered = null; } return; }
  const el = e.target.closest("[data-el]");
  if (el === hovered) return;
  if (hovered) hovered.classList.remove("el-hover");
  hovered = el && !el.closest(".inspector") ? el : null;
  if (hovered) hovered.classList.add("el-hover");
  const h = document.getElementById("edit-hover");
  if (h) h.textContent = hovered ? (NAMES[hovered.getAttribute("data-el")] || hovered.getAttribute("data-el")) : "";
}, true);
document.addEventListener("contextmenu", (e) => {
  const t = e.target;
  if (!(t.tagName === "INPUT" || t.tagName === "TEXTAREA")) e.preventDefault();
});
document.addEventListener("dragstart", (e) => { if (e.target.tagName === "IMG") e.preventDefault(); });
await new Promise(() => {});
"#;

#[derive(Deserialize, Debug)]
struct JsMsg {
    k: String,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    v: Option<String>,
}

fn handle_key(st: AppState, m: JsMsg) {
    let step_s = st.settings.peek().seek_step_s;
    let vol_step = st.settings.peek().volume_step;
    match (m.k.as_str(), m.ctrl) {
        ("Space", _) => toggle_play(st),
        ("ArrowLeft", true) => prev(st),
        ("ArrowRight", true) => next(st, true),
        ("ArrowLeft", false) => seek_rel(st, -(step_s * 1000.0) as i64),
        ("ArrowRight", false) => seek_rel(st, (step_s * 1000.0) as i64),
        ("ArrowUp", _) => {
            let v = st.settings.peek().volume;
            set_volume(st, v + vol_step);
        }
        ("ArrowDown", _) => {
            let v = st.settings.peek().volume;
            set_volume(st, v - vol_step);
        }
        ("KeyM", _) => toggle_mute(st),
        ("KeyS", false) => toggle_shuffle(st),
        ("KeyR", false) => cycle_repeat(st),
        ("KeyL", false) => {
            let mut s = st.settings;
            let v = !s.peek().lyrics_open;
            s.write().lyrics_open = v;
        }
        ("KeyQ", false) => {
            let mut s = st.settings;
            let v = !s.peek().queue_open;
            s.write().queue_open = v;
        }
        ("KeyF", true) | ("Slash", _) => {
            navigate(st, Route::Search);
            document::eval("setTimeout(() => document.getElementById('search-input')?.focus(), 60);");
        }
        ("KeyF", false) => {
            let mut f = st.fullscreen;
            let v = !*f.peek();
            f.set(v);
        }
        ("KeyE", true) => {
            let mut e = st.edit_mode;
            let v = !*e.peek();
            e.set(v);
            if !v {
                let mut s = st.selected_el;
                s.set(None);
            }
        }
        ("Comma", true) => navigate(st, Route::Settings(SettingsTab::Sound)),
        ("F11", _) => {
            let w = window();
            let fs = w.fullscreen().is_some();
            w.set_fullscreen(!fs);
        }
        ("Escape", _) => {
            let (mut menu, mut modal, mut sel, mut edit, mut fs, mut eq) = (st.menu, st.modal, st.selected_el, st.edit_mode, st.fullscreen, st.eq_open);
            if menu.peek().is_some() {
                menu.set(None);
            } else if modal.peek().is_some() {
                modal.set(None);
            } else if sel.peek().is_some() {
                sel.set(None);
            } else if *edit.peek() {
                edit.set(false);
            } else if *eq.peek() {
                eq.set(false);
            } else if *fs.peek() {
                if window().fullscreen().is_some() {
                    window().set_fullscreen(false);
                }
                fs.set(false);
            } else if window().fullscreen().is_some() {
                window().set_fullscreen(false);
            }
        }
        ("__blur", _) => {
            let mut f = st.focused;
            f.set(false);
        }
        ("__focus", _) => {
            let mut f = st.focused;
            f.set(true);
        }
        ("__hidden", _) => {
            let mut h = st.hidden;
            h.set(true);
        }
        ("__visible", _) => {
            let mut h = st.hidden;
            h.set(false);
            if *st.reopening.peek() {
                spawn(async move {
                    tokio::time::sleep(Duration::from_millis(520)).await;
                    let mut r = st.reopening;
                    r.set(false);
                });
            }
        }
        ("__select", _) => {
            if let Some(id) = m.v {
                let mut s = st.selected_el;
                s.set(Some(id));
            }
        }
        _ => {}
    }
}

/// Отложенное сохранение: пишем на диск только после паузы в изменениях.
fn use_autosave<T: Clone + PartialEq + Send + 'static>(sig: Signal<T>, delay_ms: u64, save: fn(&T)) {
    let gen = use_hook(|| Rc::new(Cell::new(0u64)));
    use_effect(move || {
        let v = sig.read().clone();
        let g = gen.get() + 1;
        gen.set(g);
        if g == 1 {
            return; // первый запуск — это загрузка, а не изменение
        }
        let gen2 = gen.clone();
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if gen2.get() == g {
                let v2 = v.clone();
                std::thread::spawn(move || save(&v2));
            }
        });
    });
}

fn apply_window_effect(effect: WindowEffect, dark: bool) {
    let w = window();
    let win: &dioxus::desktop::tao::window::Window = &w.window;
    let _ = window_vibrancy::clear_acrylic(win);
    let _ = window_vibrancy::clear_blur(win);
    let _ = window_vibrancy::clear_mica(win);
    let res = match effect {
        WindowEffect::None => Ok(()),
        WindowEffect::Mica => window_vibrancy::apply_mica(win, Some(dark)),
        WindowEffect::Acrylic => window_vibrancy::apply_acrylic(win, Some((12, 10, 26, 96))),
        WindowEffect::Blur => window_vibrancy::apply_blur(win, Some((12, 10, 26, 80))),
    };
    if let Err(e) = res {
        tracing::warn!("эффект окна {effect:?} недоступен: {e}");
    }
}

#[component]
pub fn App() -> Element {
    let st = use_context_provider(|| {
        let settings = Settings::load();
        let themes = theme::load_all();
        let theme = themes.iter().find(|t| t.id == settings.theme).cloned().unwrap_or_else(|| themes[0].clone());
        let local = LocalIndex::load().tracks();
        AppState {
            settings: Signal::new(settings),
            theme: Signal::new(theme),
            themes: Signal::new(themes),
            library: Signal::new(LibraryData::load()),
            local: Signal::new(local),
            local_scanning: Signal::new(false),
            route: Signal::new(Route::Home),
            back: Signal::new(Vec::new()),
            forward: Signal::new(Vec::new()),
            queue: Signal::new(Queue::default()),
            now: Signal::new(None),
            load_id: Signal::new(0),
            pstate: Signal::new(PlaybackState::Idle),
            position: Signal::new(0),
            duration: Signal::new(0),
            buffered: Signal::new(0.0),
            device: Signal::new(String::new()),
            preview: Signal::new(false),
            preloaded: Signal::new(Preloaded::default()),
            restore_pos: Signal::new(None),
            lyrics: Signal::new(LyricsState::Idle),
            lyrics_offset: Signal::new(0),
            fullscreen: Signal::new(false),
            eq_open: Signal::new(false),
            edit_mode: Signal::new(false),
            selected_el: Signal::new(None),
            toasts: Signal::new(Vec::new()),
            menu: Signal::new(None),
            modal: Signal::new(None),
            accent: Signal::new(None),
            focused: Signal::new(true),
            hidden: Signal::new(false),
            level: Signal::new(0.0),
            sc_ready: Signal::new(svc().sc.configured()),
            sc_user: Signal::new(svc().sc.user()),
            closing: Signal::new(false),
            reopening: Signal::new(false),
            search_query: Signal::new(String::new()),
            errors_in_row: Signal::new(0),
            related: Signal::new(None),
        }
    });

    // ── медиа (обложки, файлы фона) прямо из Rust, тем же origin ──
    use_asset_handler("media", move |req, responder| svc().media.handle(req, responder));

    // ── старт: восстановить сессию, обновить локальную библиотеку ──
    let mut opening = use_signal(|| true);
    let mut maximized = use_signal(|| false);
    use_hook(move || {
        restore_session(st);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            opening.set(false);
        });
        spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if !st.settings.peek().music_folders.is_empty() || (cfg!(debug_assertions) && std::env::var("NEBULA_EXTRA_MUSIC_DIR").is_ok()) {
                pages::rescan(st, false);
            }
            let limit = st.settings.peek().image_cache_mb;
            let _ = bg(async move { tokio::task::spawn_blocking(move || crate::media::trim_cache(limit)).await }).await;
        });
    });

    // ── глобальный JS: клавиши, фокус, режим редактирования ──
    use_future(move || async move {
        let names: serde_json::Map<String, serde_json::Value> = ELEMENTS.iter().map(|e| (e.id.to_string(), serde_json::Value::String(e.name.to_string()))).collect();
        let js = BOOT_JS.replace("__NAMES__", &serde_json::Value::Object(names).to_string());
        let mut ev = document::eval(&js);
        while let Ok(msg) = ev.recv::<JsMsg>().await {
            handle_key(st, msg);
        }
    });

    // ── медиаклавиши клавиатуры (работают и в фоне) ──
    let _ = use_global_shortcut("MediaPlayPause", move |s| if s == HotKeyState::Pressed { toggle_play(st) });
    let _ = use_global_shortcut("MediaTrackNext", move |s| if s == HotKeyState::Pressed { next(st, true) });
    let _ = use_global_shortcut("MediaTrackPrevious", move |s| if s == HotKeyState::Pressed { prev(st) });
    let _ = use_global_shortcut("MediaStop", move |s| if s == HotKeyState::Pressed { svc().player.pause() });

    // ── события окна: развёрнуто ли, закрытие через Alt+F4 ──
    use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if let Event::WindowEvent { event, .. } = event {
            match event {
                WindowEvent::Resized(_) => {
                    let m = window().is_maximized();
                    if *maximized.peek() != m {
                        maximized.set(m);
                    }
                }
                WindowEvent::CloseRequested => {
                    st.settings.peek().save();
                    st.library.peek().save();
                    save_session(st);
                }
                _ => {}
            }
        }
    });

    // ── события плеера ──
    use_future(move || async move {
        let Some(mut rx) = svc().player.take_events() else { return };
        while let Some(ev) = rx.recv().await {
            let current = *st.load_id.peek();
            match ev {
                PlayerEvent::Ended { load_id } if load_id == current => next(st, false),
                PlayerEvent::Error { load_id, message } if load_id == current => {
                    let title = st.now.peek().as_ref().map(|t| t.title.clone()).unwrap_or_default();
                    toast(st, ToastKind::Error, format!("«{title}»: {message}"));
                    let mut p = st.pstate;
                    p.set(PlaybackState::Error);
                    skip_after_error(st, load_id);
                }
                PlayerEvent::Started { load_id } if load_id == current => {
                    let mut e = st.errors_in_row;
                    e.set(0);
                }
                PlayerEvent::DeviceChanged(name) => {
                    let mut d = st.device;
                    d.set(name);
                }
                _ => {}
            }
        }
    });

    // ── опрос состояния плеера: 10 Гц во время игры, ~3 Гц в паузе ──
    use_future(move || async move {
        let mut tick: u64 = 0;
        loop {
            let active = st.pstate.peek().is_active();
            tokio::time::sleep(Duration::from_millis(if active { 100 } else { 330 })).await;
            tick += 1;
            let snap = svc().player.snapshot();
            let (mut ps, mut pos, mut dur, mut buf) = (st.pstate, st.position, st.duration, st.buffered);
            if snap.load_id != 0 && snap.load_id == *st.load_id.peek() {
                // ошибку/загрузку UI выставляет сам, плеер лишь подтверждает
                if *ps.peek() != snap.state && !(snap.state == PlaybackState::Idle) {
                    ps.set(snap.state);
                }
                if *pos.peek() != snap.position_ms && snap.state != PlaybackState::Loading {
                    pos.set(snap.position_ms);
                }
                if snap.duration_ms > 0 && *dur.peek() != snap.duration_ms {
                    dur.set(snap.duration_ms);
                }
                let b = (snap.buffered * 100.0).round() / 100.0;
                if (*buf.peek() - b).abs() > f32::EPSILON {
                    buf.set(b);
                }
            }
            let s = st.settings.peek();
            let glow = s.beat_glow && !s.low_power;
            drop(s);
            if glow && snap.state == PlaybackState::Playing {
                let lv = (svc().player.level() * 20.0).round() / 20.0;
                let mut l = st.level;
                if (*l.peek() - lv).abs() > f32::EPSILON {
                    l.set(lv);
                }
            } else if *st.level.peek() != 0.0 {
                let mut l = st.level;
                l.set(0.0);
            }
            if active && tick % 5 == 0 {
                maybe_preload(st);
            }
            if tick % 120 == 0 && st.now.peek().is_some() {
                save_session(st);
            }
        }
    });

    // ── применение настроек к движку и окну (только при реальном изменении) ──
    let dsp = use_memo(move || st.settings.read().dsp());
    use_effect(move || svc().player.set_dsp(dsp()));
    let ahead = use_memo(move || st.settings.read().read_ahead_s);
    use_effect(move || svc().player.set_read_ahead(ahead() as u64 * 1000));
    let device = use_memo(move || st.settings.read().output_device.clone());
    let first_device = use_hook(|| Rc::new(Cell::new(true)));
    use_effect(move || {
        let d = device();
        if first_device.replace(false) {
            return;
        }
        svc().player.set_device(d);
    });
    let cache_mb = use_memo(move || st.settings.read().image_cache_mb);
    use_effect(move || svc().media.set_limit_mb(cache_mb()));
    let zoom = use_memo(move || st.settings.read().ui_scale);
    use_effect(move || window().set_zoom_level(zoom() as f64));
    let on_top = use_memo(move || st.settings.read().always_on_top);
    use_effect(move || window().set_always_on_top(on_top()));
    let effect = use_memo(move || (st.settings.read().window_effect, st.theme.read().dark));
    use_effect(move || {
        let (e, dark) = effect();
        apply_window_effect(e, dark);
    });
    let dyn_accent = use_memo(move || st.settings.read().dynamic_accent);
    use_effect(move || {
        if !dyn_accent() {
            let mut a = st.accent;
            if a.peek().is_some() {
                a.set(None);
            }
        }
    });

    // ── автосохранение ──
    use_autosave(st.settings, 700, |s: &Settings| s.save());
    use_autosave(st.library, 1200, |l: &LibraryData| l.save());
    let last_theme = use_hook(|| Rc::new(RefCell::new(None::<theme::Theme>)));
    use_effect(move || {
        let t = st.theme.read().clone();
        let prev = last_theme.replace(Some(t.clone()));
        // переключение темы — не правка; сохраняем только изменения той же темы
        if let Some(p) = prev {
            if p.id == t.id && p != t {
                let mut themes = st.themes;
                if let Some(slot) = themes.write().iter_mut().find(|x| x.id == t.id) {
                    *slot = t.clone();
                }
                let t2 = t.clone();
                spawn(async move {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    if *st.theme.peek() == t2 {
                        std::thread::spawn(move || theme::save(&t2));
                    }
                });
            }
        }
    });

    // ── CSS темы ──
    let css = use_memo(move || {
        let t = st.theme.read();
        let s = st.settings.read();
        let accent = st.accent.read();
        t.css(&s, accent.as_deref())
    });

    // ── раскладка ──
    let layout = st.theme.read().layout.clone();
    let has_track = st.now.read().is_some();
    let (lyrics_open, queue_open, pause_bg) = {
        let s = st.settings.read();
        (s.lyrics_open && s.lyrics_enabled && has_track, s.queue_open, s.pause_bg_unfocused)
    };
    let s = st.settings.read();
    let pstate = *st.pstate.read();
    let class = [
        "app",
        match layout.sidebar {
            SidebarPos::Left => "sb-left",
            SidebarPos::Right => "sb-right",
            SidebarPos::Hidden => "sb-hidden",
        },
        match layout.player {
            PlayerPos::Bottom => "pl-bottom",
            PlayerPos::Top => "pl-top",
            PlayerPos::Floating => "pl-float",
        },
        if !st.theme.read().dark { "light" } else { "" },
        if !s.animations || s.low_power { "no-anim" } else { "" },
        if s.low_power { "low-power" } else { "" },
        if pause_bg && (!*st.focused.read() || *st.hidden.read()) { "bg-paused" } else { "" },
        if *st.edit_mode.read() { "edit-mode" } else { "" },
        if *st.closing.read() { "closing" } else { "" },
        if *opening.read() && s.open_close_anim { "opening" } else { "" },
        if *st.reopening.read() && !*st.hidden.read() { "opening" } else { "" },
        if *maximized.read() { "maximized" } else { "" },
        if matches!(pstate, PlaybackState::Playing | PlaybackState::Buffering) { "is-playing" } else { "is-paused" },
    ]
    .iter()
    .filter(|c| !c.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ");
    drop(s);

    let route = st.route.read().clone();
    let page = match route.clone() {
        Route::Home => rsx! { Home {} },
        Route::Search => rsx! { Search {} },
        Route::Genres => rsx! { Genres {} },
        Route::Genre(g) => rsx! { Genre { name: g } },
        Route::Favorites => rsx! { Favorites {} },
        Route::History => rsx! { History {} },
        Route::Playlists => rsx! { Playlists {} },
        Route::LocalPlaylist(id) => rsx! { LocalPlaylistPage { id } },
        Route::ScPlaylist(info) => rsx! { ScPlaylistPage { info } },
        Route::User(user) => rsx! { UserPage { user } },
        Route::Local => rsx! { Local {} },
        Route::SoundCloud => rsx! { SoundCloudPage {} },
        Route::Settings(tab) => rsx! { SettingsPage { tab } },
    };
    // ключ должен стоять на корне rsx-блока: смена маршрута пересоздаёт страницу
    // (анимация входа, свежая загрузка данных при переходе жанр → жанр)
    let route_key = route.key();
    let page = rsx! {
        div { key: "{route_key}", class: "page-wrap", {page} }
    };
    let lyrics_left = layout.lyrics_side == Side::Left;

    rsx! {
        document::Title { "Nebula" }
        style { "{css}" }
        div { class: "{class}", "data-el": "window",
            Background {}
            div { class: "frame",
                TitleBar {}
                div { class: "body",
                    if layout.sidebar == SidebarPos::Left {
                        Sidebar {}
                    }
                    if lyrics_open && lyrics_left {
                        LyricsPanel {}
                    }
                    main { class: "content", id: "content-scroll", "data-el": "content", {page} }
                    if lyrics_open && !lyrics_left {
                        LyricsPanel {}
                    }
                    if queue_open {
                        QueuePanel {}
                    }
                    if layout.sidebar == SidebarPos::Right {
                        Sidebar {}
                    }
                }
                PlayerBar {}
            }
            if *st.fullscreen.read() {
                FullscreenLyrics {}
            }
            if *st.eq_open.read() {
                EqPanel {}
            }
            ContextMenu {}
            ModalHost {}
            Toasts {}
            if *st.edit_mode.read() {
                EditHint {}
                Inspector {}
            }
            if !*maximized.read() {
                ResizeHandles {}
            }
        }
    }
}
