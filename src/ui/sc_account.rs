//! Вход в SoundCloud из интерфейса: открыть окно входа, дождаться сессии, выйти.
//! Само окно — в `crate::sc_login`; здесь только сценарий и кнопка.

use super::actions::{bg, toast};
use super::icons::Icon;
use super::state::*;
use crate::sc_login::{self, Status, DISCOVER_URL, SIGNIN_URL};
use crate::services::svc;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::time::{Duration, Instant};

/// Как долго окно входа может быть открыто, прежде чем мы его закроем.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Открыть окно входа и дождаться, пока пользователь войдёт на сайте SoundCloud.
/// Повторный вызов во время входа просто выводит окно на передний план.
pub fn start_login(st: AppState) {
    let mut busy = st.sc_login_busy;
    if *busy.peek() {
        sc_login::request(false);
        return;
    }
    busy.set(true);
    sc_login::request(svc().sc.take_clear_on_login());
    spawn_forever(async move {
        let banned = svc().sc.banned_token();
        let started = Instant::now();
        let mut token_seen: Option<Instant> = None;
        let mut redirected = false;
        let mut tick = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(700)).await;
            tick += 1;
            let (status, error) = sc_login::status();
            match status {
                Status::Open => {}
                Status::Opening if started.elapsed() < Duration::from_secs(10) => continue,
                Status::Opening | Status::Failed => {
                    sc_login::close();
                    toast(st, ToastKind::Error, format!("Не удалось открыть окно входа: {}", error.unwrap_or_else(|| "WebView2 не ответил".into())));
                    break;
                }
                Status::Closed => {
                    toast(st, ToastKind::Info, "Вход в SoundCloud отменён");
                    break;
                }
                Status::Idle => break,
            }
            if started.elapsed() > LOGIN_TIMEOUT {
                sc_login::close();
                toast(st, ToastKind::Info, "Окно входа закрыто: вход не завершён за 20 минут");
                break;
            }

            let probe = sc_login::probe().await.unwrap_or_default();
            if probe.href.as_deref() == Some("about:blank") {
                // данные прошлой сессии стёрты — открываем страницу входа
                sc_login::navigate(SIGNIN_URL);
                continue;
            }
            let usable = |t: &String| !t.is_empty() && Some(t) != banned.as_ref();
            let mut token = probe.token.filter(usable);
            if token.is_none() && tick % 4 == 0 {
                // cookie может быть недоступна из JS — спрашиваем хранилище WebView2
                token = sc_login::token_from_cookie_store().filter(usable);
            }
            let Some(token) = token else { continue };
            let seen = *token_seen.get_or_insert_with(Instant::now);

            let Some(client_id) = probe.cid else {
                // вошли, но страница без данных веб-клиента (например, secure.soundcloud.com):
                // ждём редиректа, а если его нет — открываем главную SoundCloud сами
                if !redirected && seen.elapsed() > Duration::from_millis(2500) {
                    redirected = true;
                    sc_login::navigate(DISCOVER_URL);
                } else if redirected && seen.elapsed() > Duration::from_secs(30) {
                    sc_login::close();
                    toast(st, ToastKind::Error, "Не удалось завершить вход: SoundCloud не отдал данные веб-клиента");
                    break;
                }
                continue;
            };

            sc_login::close();
            match bg(async move { svc().sc.web_login(token, client_id).await }).await {
                Some(Ok(user)) => {
                    let (mut ready, mut who) = (st.sc_ready, st.sc_user);
                    ready.set(true);
                    who.set(Some(user.clone()));
                    toast(st, ToastKind::Success, format!("Вы вошли в SoundCloud как {}", user.username));
                }
                Some(Err(e)) => toast(st, ToastKind::Error, format!("Вход не удался: {e}")),
                None => {}
            }
            break;
        }
        busy.set(false);
    });
}

pub fn logout(st: AppState) {
    svc().sc.logout();
    let (mut ready, mut who) = (st.sc_ready, st.sc_user);
    who.set(None);
    ready.set(svc().sc.configured());
    toast(st, ToastKind::Info, "Вы вышли из SoundCloud");
}

/// Вызывается из опроса состояния: если API ответил 401, сессия уже сброшена —
/// показываем это в интерфейсе (повторный вход — одна кнопка).
pub fn check_expired(st: AppState) {
    if svc().sc.take_expired() {
        let (mut ready, mut who) = (st.sc_ready, st.sc_user);
        who.set(None);
        ready.set(svc().sc.configured());
        toast(st, ToastKind::Error, "Сессия SoundCloud устарела — войдите снова");
    }
}

/// Кнопка «Войти через SoundCloud»; пока окно входа открыто, выводит его на передний план.
#[component]
pub fn LoginButton(#[props(default)] class: String) -> Element {
    let st = app_state();
    let busy = *st.sc_login_busy.read();
    rsx! {
        button {
            class: "btn primary sc-login-btn {class}",
            class: if busy { "waiting" },
            "data-el": "button",
            title: if busy { "Окно входа открыто — вернуться к нему" } else { "Войти на сайте SoundCloud" },
            onmousedown: move |e| e.stop_propagation(),
            onclick: move |e| {
                e.stop_propagation();
                start_login(st)
            },
            Icon { name: "cloud", size: 18 }
            if busy { "Жду вход в окне SoundCloud…" } else { "Войти через SoundCloud" }
        }
    }
}
