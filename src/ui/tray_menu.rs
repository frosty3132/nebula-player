//! Своё меню значка в трее. Системное меню Windows почти не настраивается,
//! поэтому меню — маленькое прозрачное окно со своей вёрсткой: тёмная
//! скруглённая панель, заголовок со значком и жирным «Nebula», серые пункты
//! с подсветкой и «Выход» внизу.
//!
//! Окно создаётся один раз, скрытым, и по правому щелчку по значку просто
//! показывается у курсора — без задержки на запуск WebView. Пункты меню
//! выполняет главное окно (очередь команд, как у островка).

use dioxus::desktop::tao::dpi::{LogicalSize, PhysicalPosition};
use dioxus::desktop::{use_wry_event_handler, window, Config, DesktopContext, WindowBuilder};
use dioxus::prelude::*;
use parking_lot::Mutex;
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// Что показывает меню: играет ли музыка и состояние переключателей.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuSnap {
    pub playing: bool,
    pub title: String,
}

pub enum TrayCmd {
    Open,
    TogglePlay,
    Next,
    Prev,
    Quit,
}

static SNAP: Mutex<Option<MenuSnap>> = Mutex::new(None);
static CMDS: Mutex<Vec<TrayCmd>> = Mutex::new(Vec::new());
/// Растёт при каждом показе: меню заново проигрывает появление.
static SHOWN: AtomicU32 = AtomicU32::new(0);
/// Меню сейчас на экране, и с какого момента (первые мгновения не
/// закрываемся: это ещё отпускают кнопку, которой открыли меню).
static OPEN_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Окно меню (размер в логических пикселях): панель прижата к правому
/// нижнему углу.
const W: f64 = 230.0;
const H: f64 = 250.0;

thread_local! {
    static WIN: RefCell<Option<DesktopContext>> = const { RefCell::new(None) };
}

pub fn publish(s: MenuSnap) {
    let mut g = SNAP.lock();
    if g.as_ref() != Some(&s) {
        *g = Some(s);
    }
}

pub fn take_commands() -> Vec<TrayCmd> {
    std::mem::take(&mut *CMDS.lock())
}

fn send(c: TrayCmd) {
    CMDS.lock().push(c);
    hide();
}

/// Создать окно меню заранее (скрытым).
pub fn init() {
    let p = crate::paths::paths();
    let mut wb = WindowBuilder::new()
        .with_title("Nebula — меню")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_inner_size(LogicalSize::new(W, H))
        .with_focused(false)
        .with_visible(false);
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowBuilderExtWindows;
        wb = wb.with_skip_taskbar(true).with_undecorated_shadow(false);
    }
    let cfg = Config::new()
        .with_window(wb)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<meta name=\"color-scheme\" content=\"dark\"><style>{MENU_CSS}</style>"))
        .with_data_directory(&p.webview)
        .with_disable_context_menu(true)
        .with_windows_browser_args(crate::webview_args());
    let pending = window().new_window(VirtualDom::new(MenuRoot), cfg);
    spawn(async move {
        let ctx = pending.await;
        WIN.with(|w| *w.borrow_mut() = Some(ctx));
    });
}

/// Показать меню над точкой щелчка (физические пиксели экрана).
pub fn show_at(x: f64, y: f64) {
    let Some(ctx) = WIN.with(|w| w.borrow().clone()) else { return };
    let win = &ctx.window;
    let mon = win.available_monitors().find(|m| {
        let (p, s) = (m.position(), m.size());
        x >= p.x as f64 && x < (p.x + s.width as i32) as f64 && y >= p.y as f64 && y < (p.y + s.height as i32) as f64
    });
    let scale = mon.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let (w, h) = (W * scale, H * scale);
    // правый нижний угол панели — у курсора, но не за краем монитора
    let mut px = x - w + 10.0 * scale;
    let mut py = y - h - 6.0 * scale;
    if let Some(m) = mon {
        let (p, s) = (m.position(), m.size());
        px = px.clamp(p.x as f64, (p.x + s.width as i32) as f64 - w);
        py = py.clamp(p.y as f64, (p.y + s.height as i32) as f64 - h);
    }
    win.set_inner_size(LogicalSize::new(W, H));
    win.set_outer_position(PhysicalPosition::new(px.round() as i32, py.round() as i32));
    SHOWN.fetch_add(1, Ordering::Relaxed);
    *OPEN_AT.lock() = Some(std::time::Instant::now());
    win.set_visible(true);
    win.set_focus();
}

pub fn hide() {
    *OPEN_AT.lock() = None;
    if let Some(ctx) = WIN.with(|w| w.borrow().clone()) {
        ctx.window.set_visible(false);
    }
}

/// Закрыть окно меню (выход из приложения).
pub fn close() {
    if let Some(ctx) = WIN.with(|w| w.borrow_mut().take()) {
        window().close_window(ctx.id());
    }
}

/// Нажата ли сейчас какая-нибудь кнопка мыши.
fn mouse_down() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
        // SAFETY: только чтение состояния кнопок
        unsafe { [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|k| GetAsyncKeyState(*k as i32) as u16 & 0x8000 != 0) }
    }
    #[cfg(not(windows))]
    false
}

/// Курсор над окном меню.
fn cursor_in_window() -> bool {
    let w = window();
    let win = &w.window;
    let (Ok(c), Ok(p)) = (win.cursor_position(), win.outer_position()) else { return true };
    let s = win.outer_size();
    c.x >= p.x as f64 && c.y >= p.y as f64 && c.x < (p.x + s.width as i32) as f64 && c.y < (p.y + s.height as i32) as f64
}

#[component]
fn MenuRoot() -> Element {
    let my_id = use_hook(|| window().id());
    let mut snap = use_signal(MenuSnap::default);
    let mut shown = use_signal(|| 0u32);
    // сводка и «показали заново»
    use_future(move || async move {
        loop {
            if let Some(s) = SNAP.lock().clone() {
                if *snap.peek() != s {
                    snap.set(s);
                }
            }
            // щелчок мышью где угодно вне меню закрывает его — даже если окну
            // не дали фокус (тогда «потерю фокуса» Windows не присылает)
            let opened = *OPEN_AT.lock();
            // Dioxus показывает каждое новое окно, когда оно загрузилось, —
            // меню же должно быть видно, только пока его открыли
            if opened.is_none() && window().window.is_visible() {
                window().set_visible(false);
            }
            if opened.is_some_and(|t| t.elapsed() > Duration::from_millis(250)) && mouse_down() && !cursor_in_window() {
                hide();
            }
            let g = SHOWN.load(Ordering::Relaxed);
            if *shown.peek() != g {
                shown.set(g);
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    });
    // щёлкнули мимо — меню закрывается, как системное
    use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if let Event::WindowEvent { window_id, event: WindowEvent::Focused(false), .. } = event {
            if *window_id == my_id {
                hide();
            }
        }
    });
    let s = snap.read().clone();
    let k = shown();
    rsx! {
        div {
            class: "tm-root",
            tabindex: "0",
            onclick: move |_| hide(),
            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Escape { hide() },
            div { key: "{k}", class: "tm-wrap",
                div { class: "tm-panel tm-main", onclick: move |e| e.stop_propagation(),
                    button { class: "tm-head", title: "Открыть Nebula", onclick: move |_| send(TrayCmd::Open),
                        span { class: "tm-logo", dangerous_inner_html: super::icons::LOGO_SVG }
                        span { class: "tm-name", "Nebula" }
                    }
                    if !s.title.is_empty() {
                        div { class: "tm-now", "{s.title}" }
                    }
                    div { class: "tm-sep" }
                    button { class: "tm-item", onclick: move |_| send(TrayCmd::TogglePlay),
                        span { class: "tm-label", if s.playing { "Пауза" } else { "Играть" } }
                    }
                    button { class: "tm-item", onclick: move |_| send(TrayCmd::Next),
                        span { class: "tm-label", "Следующий трек" }
                    }
                    button { class: "tm-item", onclick: move |_| send(TrayCmd::Prev),
                        span { class: "tm-label", "Предыдущий трек" }
                    }
                    div { class: "tm-sep" }
                    button { class: "tm-item", onclick: move |_| send(TrayCmd::Quit),
                        span { class: "tm-label", "Выход" }
                    }
                }
            }
        }
    }
}

const MENU_CSS: &str = r#"
html, body { margin: 0; height: 100%; background: transparent !important; overflow: hidden; user-select: none;
  font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; -webkit-font-smoothing: antialiased; }
#main, .tm-root { height: 100%; }
.tm-root { outline: none; position: relative; }
button { border: none; background: none; color: inherit; font: inherit; cursor: default; padding: 0; text-align: left; }
.icon { display: block; }
.tm-wrap { position: absolute; right: 6px; bottom: 6px; display: flex; align-items: flex-end; gap: 4px; }
.tm-panel {
  width: 196px; padding: 4px 0; border-radius: 8px;
  background: #2b2b2b; border: 1px solid #3c3c3c;
  box-shadow: 0 8px 24px rgba(0, 0, 0, .45), 0 2px 6px rgba(0, 0, 0, .3);
  animation: tm-in 160ms cubic-bezier(.2, .9, .3, 1) both;
}
.tm-sub { width: 230px; margin-bottom: 44px; animation: tm-sub-in 150ms cubic-bezier(.2, .9, .3, 1) both; }
@keyframes tm-in { from { opacity: 0; translate: 0 6px; } }
@keyframes tm-sub-in { from { opacity: 0; translate: 6px 0; } }
.tm-head { width: 100%; display: flex; align-items: center; gap: 10px; padding: 9px 14px 8px; color: #fff; }
.tm-logo { width: 18px; height: 18px; display: grid; --accent: #f2f2f2; --accent2: #9a9a9a; }
.tm-logo svg { width: 100%; height: 100%; }
.tm-name { font-size: 13.5px; font-weight: 700; letter-spacing: .01em; }
.tm-now { margin: -4px 14px 6px 42px; font-size: 11px; color: #7d7d7d; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tm-sep { height: 1px; margin: 4px 0; background: #3c3c3c; }
.tm-item { width: 100%; height: 32px; display: flex; align-items: center; gap: 10px; padding: 0 14px; color: #a9a9a9; font-size: 13px; }
.tm-item:hover, .tm-item.open, .tm-head:hover { background: #383838; color: #fff; }
.tm-label { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.tm-arrow { color: #8a8a8a; font-size: 12px; }
.tm-check { width: 14px; display: grid; place-items: center; color: transparent; }
.tm-check.on { color: #e8e8e8; }
"#;
