//! Окно входа в SoundCloud: отдельное нативное окно (tao + wry) со страницей входа SoundCloud.
//!
//! Окна Dioxus для этого не подходят: любой http(s)-переход в них уходит в системный браузер.
//! Пользователь входит как на сайте (пароль, Google, Apple — всё на стороне SoundCloud);
//! Nebula читает в этом окне только cookie сессии `oauth_token` и client_id веб-клиента,
//! после чего окно закрывается. Профиль WebView2 отдельный: cookies переживают перезапуск,
//! поэтому повторный вход занимает один клик.
//!
//! Окно создаётся в обработчике событий цикла (`pump`), где доступен `EventLoopWindowTarget`;
//! UI лишь ставит флаг запроса через [`request`].

use crate::paths::paths;
use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::tao::event::{Event, WindowEvent};
use dioxus::desktop::tao::event_loop::EventLoopWindowTarget;
use dioxus::desktop::tao::window::{Theme, Window, WindowBuilder};
use dioxus::desktop::wry::{self, Rect, WebContext, WebView, WebViewBuilder};
use serde::Deserialize;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

pub const SIGNIN_URL: &str = "https://soundcloud.com/signin";
pub const DISCOVER_URL: &str = "https://soundcloud.com/discover";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Status {
    #[default]
    Idle,
    /// Запрошено: окно появится на ближайшем событии цикла.
    Opening,
    Open,
    /// Пользователь закрыл окно сам.
    Closed,
    Failed,
}

#[derive(Default)]
struct State {
    requested: bool,
    clear_data: bool,
    status: Status,
    error: Option<String>,
    // порядок полей = порядок удаления: сначала WebView, потом окно
    webview: Option<Rc<WebView>>,
    window: Option<Window>,
    context: Option<WebContext>,
}

impl State {
    fn take_window(&mut self) -> (Option<Rc<WebView>>, Option<Window>, Option<WebContext>) {
        (self.webview.take(), self.window.take(), self.context.take())
    }
}

thread_local! {
    static LOGIN: RefCell<State> = RefCell::new(State::default());
}

/// Что удалось прочитать на странице SoundCloud.
#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct Probe {
    pub token: Option<String>,
    pub cid: Option<String>,
    pub href: Option<String>,
}

const PROBE_JS: &str = r#"(() => {
  const m = document.cookie.match(/(?:^|;\s*)oauth_token=([^;]+)/);
  let cid = null;
  try {
    const h = window.__sc_hydration || [];
    const a = h.find(x => x && x.hydratable === 'apiClient');
    cid = (a && a.data && a.data.id) || null;
  } catch (e) {}
  return { token: m ? decodeURIComponent(m[1]) : null, cid: cid, href: location.href };
})()"#;

/// Запросить окно входа (из обработчика UI). `clear` — сначала стереть данные прошлого входа.
/// Если окно уже открыто — просто вывести его на передний план.
pub fn request(clear: bool) {
    LOGIN.with(|l| {
        let mut l = l.borrow_mut();
        if let Some(w) = &l.window {
            w.set_focus();
            return;
        }
        l.requested = true;
        l.clear_data |= clear;
        l.status = Status::Opening;
        l.error = None;
    });
}

pub fn status() -> (Status, Option<String>) {
    LOGIN.with(|l| {
        let l = l.borrow();
        (l.status, l.error.clone())
    })
}

/// Закрыть окно входа (после успешного входа или по таймауту).
pub fn close() {
    let taken = LOGIN.with(|l| {
        let mut l = l.borrow_mut();
        l.requested = false;
        l.status = Status::Idle;
        l.take_window()
    });
    drop(taken);
}

/// Копия ссылки на WebView: заимствование `RefCell` отпускается сразу, поэтому
/// вызовы WebView2 (в том числе с вложенной прокачкой сообщений) не конфликтуют с `pump`.
fn webview() -> Option<Rc<WebView>> {
    LOGIN.with(|l| l.borrow().webview.clone())
}

pub fn navigate(url: &str) {
    if let Some(w) = webview() {
        let _ = w.load_url(url);
    }
}

/// Прочитать cookie сессии, client_id веб-клиента и адрес страницы.
pub async fn probe() -> Option<Probe> {
    let w = webview()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    w.evaluate_script_with_callback(PROBE_JS, move |res| {
        if let Some(tx) = tx.lock().ok().and_then(|mut g| g.take()) {
            let _ = tx.send(res);
        }
    })
    .ok()?;
    drop(w);
    let res = tokio::time::timeout(Duration::from_secs(3), rx).await.ok()?.ok()?;
    parse_probe(&res)
}

pub fn parse_probe(json: &str) -> Option<Probe> {
    serde_json::from_str(json).ok()
}

/// Запасной путь, если cookie недоступна из JS (HttpOnly). WebView2 отдаёт cookies
/// с вложенной прокачкой сообщений, поэтому вызываем только из задачи, не из `pump`.
pub fn token_from_cookie_store() -> Option<String> {
    let w = webview()?;
    let cookies = w.cookies_for_url("https://soundcloud.com").ok()?;
    cookies.iter().find(|c| c.name() == "oauth_token").map(|c| c.value().to_string()).filter(|v| !v.is_empty())
}

/// Что делать с `window.open` на странице входа.
#[derive(Debug, PartialEq, Eq)]
enum Popup {
    /// Окно входа через Google / Facebook / Apple или сам SoundCloud — открыть.
    SignIn,
    /// Обычная внешняя ссылка (условия использования и т. п.) — в системный браузер.
    External,
    /// Не http(s) — игнорировать.
    Blocked,
}

/// Домены, которым нужно всплывающее окно для входа.
const SIGN_IN_HOSTS: &[&str] = &["soundcloud.com", "google.com", "facebook.com", "apple.com"];

fn popup_kind(url: &str) -> Popup {
    // Google сначала открывает пустое окно и уже потом задаёт ему адрес
    if url.is_empty() || url == "about:blank" {
        return Popup::SignIn;
    }
    let Ok(u) = reqwest::Url::parse(url) else { return Popup::Blocked };
    if !matches!(u.scheme(), "https" | "http") {
        return Popup::Blocked;
    }
    let host = u.host_str().unwrap_or_default().to_ascii_lowercase();
    let sign_in = SIGN_IN_HOSTS.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")));
    if sign_in {
        Popup::SignIn
    } else {
        Popup::External
    }
}

/// Открыть http(s)-ссылку в браузере по умолчанию.
fn open_in_browser(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

fn open<T: 'static>(target: &EventLoopWindowTarget<T>, clear: bool) -> Result<(), String> {
    let window = WindowBuilder::new()
        .with_title("Вход в SoundCloud — Nebula")
        // страница SoundCloud рассчитана на десктоп: уже ~960 px появляется горизонтальная прокрутка
        .with_inner_size(LogicalSize::new(1000.0, 720.0))
        .with_min_inner_size(LogicalSize::new(420.0, 520.0))
        .with_theme(Some(Theme::Dark))
        .with_window_icon(crate::app_icon())
        .build(target)
        .map_err(|e| format!("окно входа: {e}"))?;

    let mut context = WebContext::new(Some(paths().cache.join("webview-login")));
    #[allow(unused_mut)]
    let mut builder = WebViewBuilder::new_with_web_context(&mut context)
        // после «Выйти» сначала стираем данные прошлой сессии, страницу входа откроет задача входа
        .with_url(if clear { "about:blank" } else { SIGNIN_URL })
        .with_devtools(cfg!(debug_assertions))
        // «Продолжить с Google / Facebook / Apple» открывает окно входа через window.open.
        // Без обработчика wry блокирует любые всплывающие окна, и SoundCloud пишет
        // «Please enable popup windows». Окна входа разрешаем (как обычный браузер),
        // прочие ссылки страницы открываем в системном браузере.
        .with_new_window_req_handler(|url, _features| match popup_kind(&url) {
            Popup::SignIn => wry::NewWindowResponse::Allow,
            Popup::External => {
                open_in_browser(&url);
                wry::NewWindowResponse::Deny
            }
            Popup::Blocked => wry::NewWindowResponse::Deny,
        });
    // Только для отладочной сборки: DevTools Protocol окна входа на порту основного + 1.
    #[cfg(debug_assertions)]
    if let Some(port) = std::env::var("NEBULA_DEVTOOLS_PORT").ok().and_then(|p| p.parse::<u16>().ok()) {
        use dioxus::desktop::wry::WebViewBuilderExtWindows;
        let p = port + 1;
        builder = builder.with_additional_browser_args(format!("--remote-debugging-port={p} --remote-allow-origins=http://127.0.0.1:{p}"));
    }
    let webview = builder.build(&window).map_err(|e| format!("WebView2: {e}"))?;
    if clear {
        let _ = webview.clear_all_browsing_data();
    }

    LOGIN.with(|l| {
        let mut l = l.borrow_mut();
        l.webview = Some(Rc::new(webview));
        l.window = Some(window);
        l.context = Some(context);
        l.status = Status::Open;
    });
    Ok(())
}

/// Вызывается из `use_wry_event_handler` на каждое событие цикла:
/// создаёт запрошенное окно и обслуживает его закрытие и изменение размера.
pub fn pump<T: 'static>(event: &Event<'_, T>, target: &EventLoopWindowTarget<T>) {
    let (create, clear) = LOGIN.with(|l| {
        let mut l = l.borrow_mut();
        let create = l.requested && l.window.is_none();
        if create {
            l.requested = false;
        }
        (create, if create { std::mem::take(&mut l.clear_data) } else { false })
    });
    if create {
        if let Err(e) = open(target, clear) {
            tracing::warn!("окно входа SoundCloud: {e}");
            LOGIN.with(|l| {
                let mut l = l.borrow_mut();
                l.status = Status::Failed;
                l.error = Some(e);
            });
        }
    }

    let Event::WindowEvent { window_id, event, .. } = event else { return };
    let ours = LOGIN.with(|l| l.borrow().window.as_ref().map(|w| w.id()) == Some(*window_id));
    if !ours {
        return;
    }
    match event {
        WindowEvent::CloseRequested => {
            let taken = LOGIN.with(|l| {
                let mut l = l.borrow_mut();
                l.status = Status::Closed;
                l.take_window()
            });
            drop(taken);
        }
        WindowEvent::Resized(size) => {
            if let Some(w) = webview() {
                let _ = w.set_bounds(Rect {
                    position: wry::dpi::LogicalPosition::new(0.0, 0.0).into(),
                    size: wry::dpi::PhysicalSize::new(size.width, size.height).into(),
                });
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_in_popups_are_allowed() {
        for url in [
            "about:blank",
            "https://accounts.google.com/o/oauth2/v2/auth?client_id=x",
            "https://www.facebook.com/v18.0/dialog/oauth?client_id=x",
            "https://appleid.apple.com/auth/authorize?client_id=x",
            "https://secure.soundcloud.com/authorize",
        ] {
            assert_eq!(popup_kind(url), Popup::SignIn, "{url}");
        }
        assert_eq!(popup_kind("https://soundcloud.com/terms-of-use"), Popup::SignIn);
        assert_eq!(popup_kind("https://help.example.org/article"), Popup::External);
        // похожие, но чужие домены не считаются окнами входа
        assert_eq!(popup_kind("https://google.com.evil.example/x"), Popup::External);
        assert_eq!(popup_kind("https://notgoogle.com/x"), Popup::External);
        assert_eq!(popup_kind("javascript:alert(1)"), Popup::Blocked);
        assert_eq!(popup_kind("file:///C:/Windows"), Popup::Blocked);
    }

    #[test]
    fn probe_parsing() {
        let p = parse_probe(r#"{"token":"2-123-456-abc","cid":"XyZ","href":"https://soundcloud.com/discover"}"#).unwrap();
        assert_eq!(p.token.as_deref(), Some("2-123-456-abc"));
        assert_eq!(p.cid.as_deref(), Some("XyZ"));
        let empty = parse_probe(r#"{"token":null,"cid":null,"href":"about:blank"}"#).unwrap();
        assert_eq!(empty, Probe { token: None, cid: None, href: Some("about:blank".into()) });
        assert!(parse_probe("null").is_none());
    }
}
