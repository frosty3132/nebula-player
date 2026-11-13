//! «Хром» окна: собственный заголовок (перетаскивание, кнопки окна с анимацией
//! закрытия/сворачивания), боковое меню и невидимые ручки изменения размера.

use super::actions::{go_back, go_forward, navigate, save_session};
use super::icons::{Icon, LOGO_SVG};
use super::player_bar::Island;
use super::state::*;
use super::widgets::IconBtn;
use crate::theme::{Entry, SidebarPos, NAV_ITEMS};
use dioxus::desktop::tao::window::ResizeDirection;
use dioxus::desktop::window;
use dioxus::prelude::*;

/// Крестик: с настройкой «Сворачивать в трей» окно прячется к часам у
/// панели задач (музыка играет дальше), иначе приложение закрывается.
pub fn close_app(state: AppState) {
    if state.settings.peek().close_to_tray {
        hide_to_tray(state);
    } else {
        quit_app(state);
    }
}

/// Запомнить размер и положение окна и сохранить всё на диск.
fn save_state(state: AppState) {
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
}

/// Спрятать окно в трей. Состояние сохраняем сразу — на случай, если
/// Windows завершит приложение, пока оно в фоне.
pub fn hide_to_tray(state: AppState) {
    save_state(state);
    window().set_visible(false);
}

/// Закрыть приложение совсем — сразу, без анимации ухода.
pub fn quit_app(state: AppState) {
    save_state(state);
    // островок — отдельное окно: без него приложение не завершилось бы
    super::desk_island::close_now();
    super::tray_menu::close();
    // с «трей вместо закрытия» окно лишь пряталось бы
    window().set_close_behavior(dioxus::desktop::WindowCloseBehaviour::WindowCloses);
    window().close();
}

/// Системные анимации окна Windows. Обычно выключены: при развёртывании
/// Windows растягивает снимок прозрачного окна, и оно вспыхивает белым. На
/// время сворачивания и возвращения из панели задач их включаем — окно
/// сворачивается и разворачивается как у обычных приложений.
pub fn set_window_transitions(enabled: bool) {
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_TRANSITIONS_FORCEDISABLED};
        let hwnd = window().window.hwnd();
        let off: i32 = (!enabled).into();
        // SAFETY: hwnd — живое окно приложения, буфер — один BOOL (i32)
        unsafe {
            DwmSetWindowAttribute(hwnd as _, DWMWA_TRANSITIONS_FORCEDISABLED as u32, (&off as *const i32).cast(), 4);
        }
    }
    #[cfg(not(windows))]
    let _ = enabled;
}

/// Убрать системную рамку Windows 11: светлую обводку и собственное скругление
/// окна. Углы у Nebula круглее системных, и из-за них выглядывала белая рамка;
/// окно прозрачное, скругление рисует сама страница.
pub fn style_window_frame() {
    set_window_transitions(false);
    // при сворачивании Windows дорисовывала поверх окна классический заголовок
    // «как в Windows 7», и он мелькал в анимации разворачивания
    #[cfg(windows)]
    super::desk_island::no_classic_frame(&window().window, false);
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::Graphics::Dwm::{
            DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND,
        };
        let hwnd = window().window.hwnd();
        let none: u32 = DWMWA_COLOR_NONE;
        let corners: i32 = DWMWCP_DONOTROUND;
        // SAFETY: hwnd — живое окно приложения, буферы — COLORREF (u32) и enum (i32);
        // на Windows 10 атрибуты не поддерживаются, и вызов просто вернёт ошибку
        unsafe {
            DwmSetWindowAttribute(hwnd as _, DWMWA_BORDER_COLOR as u32, (&none as *const u32).cast(), 4);
            DwmSetWindowAttribute(hwnd as _, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&corners as *const i32).cast(), 4);
        }
    }
}

/// Развернуть/восстановить окно (системная анимация выключена — без вспышки).
pub fn toggle_maximized_quiet() {
    set_window_transitions(false);
    window().toggle_maximized();
}

/// Свернуть без включения системных переходов DWM — чтобы при разворачивании
/// за прозрачным окном не мелькала классическая рамка Windows 7.
pub fn minimize_app(_state: AppState) {
    set_window_transitions(false);
    window().set_minimized(true);
}

/// Полный экран (F11) с анимацией: окно прячется в «начальный кадр» анимации
/// ещё до переключения, иначе на долю секунды мелькал готовый кадр.
pub fn toggle_fullscreen_animated(st: AppState) {
    let w = window();
    let to_fs = w.fullscreen().is_none();
    let anim = {
        let s = st.settings.peek();
        s.animations && !s.low_power
    };
    if anim {
        let cls = if to_fs { "win-pre-grow" } else { "win-pre-shrink" };
        let _ = document::eval(&format!(
            "const a = document.querySelector('.app'); if (a) {{ a.classList.add('{cls}'); setTimeout(() => a.classList.remove('{cls}'), 700); }}"
        ));
    }
    w.set_fullscreen(to_fs);
}

/// Переключает режим интерфейса: «Музыка» (разделы, поиск, плейлисты) или
/// «Сцена» (обложка, плеер и текст на весь экран).
pub fn set_app_mode(st: AppState, stage: bool) {
    if !stage && window().fullscreen().is_some() {
        window().set_fullscreen(false);
    }
    let mut fs = st.fullscreen;
    fs.set(stage);
}

/// Переключатель «Музыка | Сцена» по центру заголовка: стеклянная капсула,
/// под выбранным пунктом скользит светлая подложка, у каждого — свой значок.
#[component]
pub fn ModeSwitch() -> Element {
    let st = app_state();
    let stage = *st.fullscreen.read();
    rsx! {
        div {
            class: if stage { "mode-switch on-stage" } else { "mode-switch" },
            onmousedown: move |e| e.stop_propagation(),
            ondoubleclick: move |e| e.stop_propagation(),
            span { class: "mode-glider" }
            button {
                class: if !stage { "mode-btn active" } else { "mode-btn" },
                title: "Музыка: разделы, поиск и плейлисты (Esc)",
                onclick: move |_| set_app_mode(st, false),
                Icon { name: "music", size: 15 }
                span { "Музыка" }
            }
            button {
                class: if stage { "mode-btn active" } else { "mode-btn" },
                title: "Сцена: обложка, плеер и текст на весь экран (F)",
                onclick: move |_| set_app_mode(st, true),
                Icon { name: "sparkles", size: 15 }
                span { "Сцена" }
            }
        }
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
    // при островке на рабочем столе встроенный островок в заголовке не нужен
    let collapsed = { let s = st.settings.read(); s.player_collapsed && !s.island_desktop };

    let mut max_sig = maximized;
    let win_btns = rsx! {
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
                    toggle_maximized_quiet();
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
    };
    // стиль «Сцена»: слева название и часы, по центру свёрнутый островок, справа
    // значки разделов, переключатель «Музыка / Сцена» по центру и кнопки окна
    if layout.stage_style {
        return rsx! {
            header {
                class: "titlebar stage-titlebar",
                "data-el": "titlebar",
                onmousedown: move |e| {
                    if e.trigger_button() == Some(dioxus::html::input_data::MouseButton::Primary) {
                        window().drag();
                    }
                },
                ondoubleclick: move |_| {
                    toggle_maximized_quiet();
                    max_sig.set(window().is_maximized());
                },
                div { class: "tb-left stage-side",
                    super::stage::BrandClock {}
                    div { class: "tb-nav",
                        IconBtn { icon: "back", title: "Назад", size: 18, class: if can_back { "" } else { "dim" }, onclick: move |_| go_back(st) }
                        IconBtn { icon: "forward", title: "Вперёд", size: 18, class: if can_fwd { "" } else { "dim" }, onclick: move |_| go_forward(st) }
                    }
                    // значки разделов — слева, сразу за названием и стрелками
                    if top_dock {
                        Dock { nav: layout.nav.clone(), route: route.clone() }
                    }
                }
                div { class: "stage-center",
                    ModeSwitch {}
                    if collapsed {
                        Island {}
                    }
                }
                div { class: "tb-right stage-side end",
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
                    {win_btns}
                }
            }
        };
    }
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
                toggle_maximized_quiet();
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
            div { class: "tb-center", ModeSwitch {} }
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
                {win_btns}
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
                        span { class: if sc_ready && st.sc_user.read().is_some() { "dot on" } else { "dot" } }
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
