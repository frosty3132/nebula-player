//! «Хром» окна: собственный заголовок (перетаскивание, кнопки окна с анимацией
//! закрытия/сворачивания), боковое меню и невидимые ручки изменения размера.

use super::actions::{go_back, go_forward, navigate, save_session};
use super::icons::{Icon, LOGO_SVG};
use super::player_bar::Island;
use super::state::*;
use super::widgets::IconBtn;
use crate::theme::{Entry, SidebarPos, NAV_ITEMS};
use dioxus::desktop::tao::window::ResizeDirection;
use dioxus::core::spawn_forever;
use dioxus::desktop::window;
use dioxus::prelude::*;
use std::time::Duration;

/// Сохранить размер/положение окна и состояние, затем закрыть с анимацией.
pub fn close_app(state: AppState) {
    let mut st = state;
    {
        let w = window();
        let scale = w.scale_factor();
        let mut s = st.settings.write();
        s.window.maximized = w.is_maximized();
        if !s.window.maximized {
            let size = w.inner_size().to_logical::<f64>(scale);
            s.window.width = size.width;
            s.window.height = size.height;
            if let Ok(pos) = w.outer_position() {
                s.window.x = Some(pos.x);
                s.window.y = Some(pos.y);
            }
        }
    }
    st.settings.peek().save();
    st.library.peek().save();
    save_session(state);
    let animate = {
        let s = st.settings.peek();
        s.open_close_anim && s.animations && !s.low_power
    };
    if animate {
        st.closing.set(true);
        spawn_forever(async move {
            tokio::time::sleep(Duration::from_millis(260)).await;
            window().close();
        });
    } else {
        window().close();
    }
}

pub fn minimize_app(state: AppState) {
    let mut st = state;
    let animate = {
        let s = st.settings.peek();
        s.open_close_anim && s.animations && !s.low_power
    };
    if animate {
        st.closing.set(true);
        spawn_forever(async move {
            tokio::time::sleep(Duration::from_millis(180)).await;
            window().set_minimized(true);
            st.closing.set(false);
            st.reopening.set(true);
        });
    } else {
        window().set_minimized(true);
    }
}

#[component]
pub fn TitleBar() -> Element {
    let st = app_state();
    let layout = st.theme.read().layout.clone();
    let route = st.route.read().clone();
    let maximized = use_signal(|| false);
    let can_back = !st.back.read().is_empty();
    let can_fwd = !st.forward.read().is_empty();
    let edit = *st.edit_mode.read();
    // «Hidden» из старых тем показываем так же, как верхнюю панель
    let top_dock = matches!(layout.sidebar, SidebarPos::Top | SidebarPos::Hidden);
    let collapsed = st.settings.read().player_collapsed;

    let mut max_sig = maximized;
    rsx! {
        header {
            class: "titlebar",
            "data-el": "titlebar",
            onmousedown: move |e| {
                if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary) {
                    window().drag();
                }
            },
            ondoubleclick: move |_| {
                window().toggle_maximized();
                max_sig.set(window().is_maximized());
            },
            div { class: "tb-left",
                // с верхней панелью логотип стоит над ней (см. tb-center)
                if !top_dock && layout.titlebar_title {
                    div { class: "tb-brand",
                        span { class: "tb-logo", dangerous_inner_html: LOGO_SVG }
                        span { class: "tb-name", "Nebula" }
                    }
                }
                div { class: "tb-nav",
                    IconBtn { icon: "back", title: "Назад", size: 18, class: if can_back { "" } else { "dim" }, onclick: move |_| go_back(st) }
                    IconBtn { icon: "forward", title: "Вперёд", size: 18, class: if can_fwd { "" } else { "dim" }, onclick: move |_| go_forward(st) }
                }
            }
            div { class: "tb-center",
                if top_dock {
                    div { class: "tb-brand above",
                        span { class: "tb-name", "Nebula" }
                    }
                }
                div { class: "tb-row",
                    // островок (свёрнутый плеер) — слева от панели разделов
                    if collapsed {
                        Island {}
                    }
                    if top_dock {
                        Dock { nav: layout.nav.clone(), route: route.clone() }
                    }
                }
            }
            div { class: "tb-right",
                IconBtn {
                    icon: "brush",
                    title: "Режим редактирования интерфейса (Ctrl+E)",
                    size: 17,
                    active: edit,
                    class: "tb-edit",
                    onclick: move |_| {
                        let mut s = st;
                        let v = !*s.edit_mode.peek();
                        s.edit_mode.set(v);
                        if !v {
                            s.selected_el.set(None);
                        }
                    },
                }
                div { class: "win-btns",
                    button {
                        class: "win-btn",
                        title: "Свернуть",
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| minimize_app(st),
                        span { class: "wb-min" }
                    }
                    button {
                        class: "win-btn",
                        title: if *maximized.read() { "Восстановить" } else { "Развернуть" },
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| {
                            window().toggle_maximized();
                            max_sig.set(window().is_maximized());
                        },
                        span { class: if *maximized.read() { "wb-restore" } else { "wb-max" } }
                    }
                    button {
                        class: "win-btn close",
                        title: "Закрыть",
                        onmousedown: move |e| e.stop_propagation(),
                        onclick: move |_| close_app(st),
                        Icon { name: "close", size: 16 }
                    }
                }
            }
        }
    }
}

/// Верхняя панель-док: значки разделов. При наведении значок слегка
/// приподнимается и светлеет, а через мгновение показывает подпись.
/// Подсветка активного раздела — одна «капля» (`.glider`), которая плавно
/// переезжает под новый значок. Всё на CSS (`scale`/`translate`) — работает
/// на GPU-композиторе; `--i` — номер значка для каскада при запуске.
#[component]
fn Dock(nav: Vec<Entry>, route: Route) -> Element {
    let st = app_state();
    let settings_active = matches!(route, Route::Settings(_));
    let items: Vec<(usize, &'static str, &'static str, &'static str)> = nav
        .iter()
        .filter(|e| e.visible)
        .filter_map(|e| NAV_ITEMS.iter().find(|n| n.0 == e.id))
        .enumerate()
        .map(|(i, n)| (i, n.0, n.1, n.2))
        .collect();
    let n = items.len();
    rsx! {
        nav { class: "dock", "data-el": "nav", "data-glide": ".dock-item.active", "data-glide-in": ".dock-icon",
            span { class: "glider dock-glider" }
            for (i , id , label , icon) in items {
                button {
                    key: "{id}",
                    class: if route.nav_id() == id { "dock-item active" } else { "dock-item" },
                    style: "--i: {i}",
                    "data-el": "nav_item",
                    "aria-label": "{label}",
                    onmousedown: move |e| e.stop_propagation(),
                    ondoubleclick: move |e| e.stop_propagation(),
                    onclick: move |_| navigate(st, Route::from_nav(id)),
                    span { class: "dock-icon", Icon { name: icon.to_string(), size: 20 } }
                    span { class: "dock-label", "{label}" }
                }
            }
            span { class: "dock-sep", style: "--i: {n}" }
            button {
                class: if settings_active { "dock-item active" } else { "dock-item" },
                style: "--i: {n + 1}",
                "data-el": "nav_item",
                "aria-label": "Настройки",
                onmousedown: move |e| e.stop_propagation(),
                ondoubleclick: move |e| e.stop_propagation(),
                onclick: move |_| navigate(st, Route::Settings(SettingsTab::Sound)),
                span { class: "dock-icon", Icon { name: "settings", size: 20 } }
                span { class: "dock-label", "Настройки" }
            }
        }
    }
}

#[component]
pub fn Sidebar() -> Element {
    let st = app_state();
    let layout = st.theme.read().layout.clone();
    let route = st.route.read().clone();
    let compact = layout.sidebar_compact;
    let playlists: Vec<(String, String, usize)> = st.library.read().playlists.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.len())).collect();
    let user = st.sc_user.read().clone();
    let sc_ready = *st.sc_ready.read();

    rsx! {
        aside { class: if compact { "sidebar compact" } else { "sidebar" }, "data-el": "sidebar",
            if layout.show_logo {
                div { class: "logo", "data-el": "logo",
                    span { class: "logo-mark", dangerous_inner_html: LOGO_SVG }
                    if !compact {
                        div { class: "logo-text",
                            span { class: "logo-name", "Nebula" }
                            span { class: "logo-sub", "galactic player" }
                        }
                    }
                }
            }
            nav { class: "nav", "data-el": "nav", "data-glide": ".nav-item.active",
                span { class: "glider nav-glider" }
                for entry in layout.nav.iter().filter(|e| e.visible) {
                    if let Some((id, label, icon)) = NAV_ITEMS.iter().find(|n| n.0 == entry.id) {
                        button {
                            key: "{id}",
                            class: if route.nav_id() == *id { "nav-item active" } else { "nav-item" },
                            "data-el": "nav_item",
                            title: "{label}",
                            onclick: move |_| navigate(st, Route::from_nav(id)),
                            Icon { name: icon.to_string(), size: 20 }
                            if !compact {
                                span { class: "nav-label", "{label}" }
                            }
                        }
                    }
                }
            }
            if !compact {
                div { class: "sb-playlists", "data-el": "sidebar_playlists",
                    div { class: "sb-head",
                        span { "Мои плейлисты" }
                        IconBtn {
                            icon: "add",
                            title: "Новый плейлист",
                            size: 16,
                            onclick: move |_| {
                                let mut m = st.modal;
                                m.set(Some(Modal::NewPlaylist { add: None }));
                            },
                        }
                    }
                    div { class: "sb-list",
                        if playlists.is_empty() {
                            div { class: "sb-empty", "Создайте первый плейлист ✦" }
                        }
                        for (id , name , count) in playlists.into_iter() {
                            button {
                                key: "{id}",
                                class: if route == Route::LocalPlaylist(id.clone()) { "sb-pl active" } else { "sb-pl" },
                                onclick: move |_| navigate(st, Route::LocalPlaylist(id.clone())),
                                Icon { name: "playlist", size: 16 }
                                span { class: "sb-pl-name", "{name}" }
                                span { class: "sb-pl-count", "{count}" }
                            }
                        }
                    }
                }
            }
            div { class: "sb-bottom",
                button {
                    class: if matches!(route, Route::Settings(_)) { "nav-item active" } else { "nav-item" },
                    "data-el": "nav_item",
                    title: "Настройки",
                    onclick: move |_| navigate(st, Route::Settings(SettingsTab::Sound)),
                    Icon { name: "settings", size: 20 }
                    if !compact {
                        span { class: "nav-label", "Настройки" }
                    }
                }
                if !compact {
                    button {
                        class: "sc-chip",
                        // без аккаунта кнопка сразу открывает окно входа SoundCloud
                        onclick: move |_| {
                            if st.sc_user.peek().is_some() {
                                navigate(st, Route::SoundCloud)
                            } else {
                                super::sc_account::start_login(st)
                            }
                        },
                        span { class: if sc_ready { "dot on" } else { "dot" } }
                        match user {
                            Some(u) => rsx! { span { "{u.username}" } },
                            None if *st.sc_login_busy.read() => rsx! { span { "Жду вход в SoundCloud…" } },
                            None => rsx! { span { "Войти в SoundCloud" } },
                        }
                    }
                }
            }
        }
    }
}

/// Невидимые края окна для изменения размера (окно без системной рамки).
#[component]
pub fn ResizeHandles() -> Element {
    let edges = [
        ("rh-n", ResizeDirection::North),
        ("rh-s", ResizeDirection::South),
        ("rh-e", ResizeDirection::East),
        ("rh-w", ResizeDirection::West),
        ("rh-ne", ResizeDirection::NorthEast),
        ("rh-nw", ResizeDirection::NorthWest),
        ("rh-se", ResizeDirection::SouthEast),
        ("rh-sw", ResizeDirection::SouthWest),
    ];
    rsx! {
        for (class , dir) in edges {
            div {
                key: "{class}",
                class: "rh {class}",
                onmousedown: move |e| {
                    e.stop_propagation();
                    if !window().is_maximized() {
                        let _ = window().drag_resize_window(dir);
                    }
                },
            }
        }
    }
}
