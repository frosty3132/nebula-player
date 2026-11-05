//! Окно «Импорт плейлиста»: сначала выбор сервиса (Яндекс Музыка или Spotify),
//! затем ссылка на плейлист. Треки сопоставляются с локальной библиотекой и
//! SoundCloud (см. `crate::import`), результат — новый локальный плейлист.
//!
//! Импорт идёт в фоне: окно можно закрыть, по готовности придёт уведомление.

use super::actions::*;
use super::icons::Icon;
use super::state::*;
use super::widgets::IconBtn;
use crate::import::{self, Service};
use crate::services::svc;
use crate::spotify::DASHBOARD_URL;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use futures_util::stream::{self, StreamExt};

/// Записать в сигнал окна, если окно ещё открыто (иначе молча пропустить).
fn put<T: 'static>(mut sig: Signal<T>, value: T) {
    if let Ok(mut w) = sig.try_write() {
        *w = value;
    }
}

#[component]
pub fn ImportDialog() -> Element {
    let service = use_signal(|| None::<Service>);
    let current = *service.read();
    match current {
        None => rsx! { ServicePicker { service } },
        Some(s) => rsx! { ImportForm { key: "{s:?}", service: s, picker: service } },
    }
}

#[component]
fn ServicePicker(service: Signal<Option<Service>>) -> Element {
    let mut service = service;
    rsx! {
        p { class: "modal-text", "Откуда перенести плейлист? Треки найдутся в вашей музыке на компьютере и на SoundCloud, остальные останутся в плейлисте заглушками." }
        div { class: "import-choices",
            button { class: "import-choice", onclick: move |_| service.set(Some(Service::Yandex)),
                span { class: "ic-badge ym", "Я" }
                span { class: "ic-text",
                    span { class: "ic-title", "Яндекс Музыка" }
                    span { class: "ic-sub", "Публичный плейлист по ссылке" }
                }
                Icon { name: "forward", size: 18 }
            }
            button { class: "import-choice", onclick: move |_| service.set(Some(Service::Spotify)),
                span { class: "ic-badge sp",
                    // три «волны» — узнаваемо, но без логотипа
                    svg { view_box: "0 0 24 24", width: "20", height: "20", "aria-hidden": "true",
                        path { d: "M5 9.2c4.6-1.5 9.6-1.1 13.7 1.1M6.2 12.9c3.7-1.1 7.6-.8 10.9.9M7.4 16.3c2.8-.8 5.6-.5 8 .7", fill: "none", stroke: "currentColor", stroke_width: "2", stroke_linecap: "round" }
                    }
                }
                span { class: "ic-text",
                    span { class: "ic-title", "Spotify" }
                    span { class: "ic-sub", "Публичный плейлист по ссылке · нужен ключ приложения" }
                }
                Icon { name: "forward", size: 18 }
            }
        }
    }
}

#[component]
fn ImportForm(service: Service, picker: Signal<Option<Service>>) -> Element {
    let st = app_state();
    let mut picker = picker;
    let mut url = use_signal(String::new);
    let busy = use_signal(|| false);
    let status = use_signal(|| None::<(bool, String)>);
    // Яндекс: необязательный OAuth-токен; Spotify: ключи приложения
    let mut token = use_signal(|| svc().yandex.token());
    let mut show_token = use_signal(|| !svc().yandex.token().is_empty());
    let mut client_id = use_signal(|| svc().spotify.client_id());
    let mut client_secret = use_signal(String::new);
    let mut keys_saved = use_signal(|| svc().spotify.configured());
    let mut edit_keys = use_signal(|| !svc().spotify.configured());

    let run = move || {
        let input = url.peek().trim().to_string();
        if input.is_empty() || *busy.peek() {
            return;
        }
        if service == Service::Spotify && !*keys_saved.peek() {
            put(status, Some((false, "Сначала сохраните Client ID и Client Secret".to_string())));
            return;
        }
        start_import(st, service, input, busy, status);
    };

    let busy_now = *busy.read();
    let (placeholder, hint) = match service {
        Service::Yandex => ("https://music.yandex.ru/users/…/playlists/…", "Ссылка вида music.yandex.ru/users/…/playlists/… или music.yandex.ru/playlists/… Плейлист должен быть публичным."),
        Service::Spotify => ("https://open.spotify.com/playlist/…", "Ссылка вида open.spotify.com/playlist/… (в Spotify: «Поделиться» → «Скопировать ссылку»). Плейлист должен быть публичным."),
    };
    rsx! {
        div { class: "import-form",
            div { class: "import-back",
                IconBtn { icon: "back", title: "К выбору сервиса", size: 18, onclick: move |_| if !*busy.peek() { picker.set(None) } }
                span { class: "import-service", "{service.label()}" }
            }
            if service == Service::Spotify && *edit_keys.read() {
                div { class: "import-keys",
                    p { class: "modal-text",
                        "Spotify отдаёт плейлисты только приложениям с ключом. Это делается один раз и бесплатно: откройте "
                        a { class: "link", href: DASHBOARD_URL, target: "_blank", "developer.spotify.com/dashboard" }
                        " → Create app (название любое, Redirect URI — http://127.0.0.1:8888/callback, отметьте Web API) → Settings — и скопируйте Client ID и Client Secret сюда. Ключи хранятся только на этом компьютере."
                    }
                    input { class: "import-input", placeholder: "Client ID", value: "{client_id}", spellcheck: false,
                        oninput: move |e: FormEvent| client_id.set(e.value()) }
                    input { class: "import-input", r#type: "password", placeholder: "Client Secret", value: "{client_secret}", spellcheck: false,
                        oninput: move |e: FormEvent| client_secret.set(e.value()) }
                    div { class: "import-row end",
                        if *keys_saved.read() {
                            button { class: "btn ghost", onclick: move |_| edit_keys.set(false), "Отмена" }
                        }
                        button { class: "btn primary",
                            disabled: client_id.read().trim().is_empty() || client_secret.read().trim().is_empty(),
                            onclick: move |_| {
                                svc().spotify.set_credentials(&client_id.peek(), &client_secret.peek());
                                keys_saved.set(true);
                                edit_keys.set(false);
                                client_secret.set(String::new());
                                put(status, Some((true, "Ключи Spotify сохранены".to_string())));
                            },
                            "Сохранить ключи"
                        }
                    }
                }
            } else {
                p { class: "modal-text", "{hint}" }
                div { class: "import-row",
                    input {
                        class: "import-input",
                        r#type: "url",
                        placeholder: "{placeholder}",
                        value: "{url}",
                        spellcheck: false,
                        autofocus: true,
                        oninput: move |e: FormEvent| url.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { run() },
                    }
                    button { class: "btn primary", disabled: busy_now || url.read().trim().is_empty(), onclick: move |_| run(),
                        Icon { name: "download", size: 18 }
                        if busy_now { "Импортирую…" } else { "Импортировать" }
                    }
                }
                match service {
                    Service::Spotify => rsx! {
                        button { class: "import-more", onclick: move |_| edit_keys.set(true), "Изменить ключи приложения Spotify" }
                    },
                    Service::Yandex => rsx! {
                        if *show_token.read() {
                            div { class: "import-row",
                                input { class: "import-input", r#type: "password", spellcheck: false,
                                    placeholder: "OAuth-токен Яндекс Музыки",
                                    value: "{token}",
                                    oninput: move |e: FormEvent| token.set(e.value()) }
                                button { class: "btn ghost", onclick: move |_| {
                                    svc().yandex.set_token(token.peek().trim());
                                    put(status, Some((true, "Токен Яндекс Музыки сохранён на этом компьютере".to_string())));
                                }, "Сохранить" }
                            }
                        } else {
                            button { class: "import-more", onclick: move |_| show_token.set(true), "Яндекс отвечает 401 или 451? Укажите OAuth-токен" }
                        }
                    },
                }
            }
            if let Some((ok, text)) = status.read().clone() {
                div { class: if ok { "note ok" } else { "note err" }, "{text}" }
            }
            if busy_now {
                p { class: "import-bg-hint", "Окно можно закрыть — импорт продолжится, по готовности придёт уведомление." }
            }
        }
    }
}

/// Скачать плейлист, сопоставить треки, сохранить как локальный плейлист.
/// Задача живёт дольше окна (`spawn_forever`): закрытие окна импорт не прерывает.
fn start_import(st: AppState, service: Service, input: String, busy: Signal<bool>, status: Signal<Option<(bool, String)>>) {
    put(busy, true);
    put(status, Some((true, format!("Загружаю плейлист из {}…", service.label()))));
    let local = st.local.peek().clone();
    let sc_ready = *st.sc_ready.peek();
    spawn_forever(async move {
        let fetched = bg(async move {
            match service {
                Service::Yandex => svc().yandex.import_playlist(&input).await,
                Service::Spotify => svc().spotify.import_playlist(&input).await,
            }
        })
        .await
        .unwrap_or_else(|| Err("Не удалось запустить импорт".into()));
        let playlist = match fetched {
            Ok(p) => p,
            Err(e) => {
                let e = if service == Service::Yandex && (e.contains("451") || e.contains("401")) {
                    "Яндекс Музыка отклонила запрос. Укажите OAuth-токен из сессии Яндекс Музыки и повторите импорт.".to_string()
                } else {
                    e
                };
                put(status, Some((false, e.clone())));
                put(busy, false);
                if !import_dialog_open(st) {
                    toast(st, ToastKind::Error, format!("Импорт из {}: {e}", service.label()));
                }
                return;
            }
        };
        let total = playlist.tracks.len();
        let title = playlist.title.clone();
        let search_note = if sc_ready { "в вашей музыке и на SoundCloud" } else { "в вашей музыке (войдите в SoundCloud, чтобы искать и там)" };
        put(status, Some((true, format!("«{title}»: ищу {total} треков {search_note}…"))));

        let matched = bg(async move {
            let sc = if sc_ready { Some(&svc().sc) } else { None };
            import::match_tracks(&playlist, &local, sc).await
        })
        .await
        .unwrap_or_default();
        let found = matched.iter().filter(|t| import::is_matched(t)).count();

        // первые обложки — заранее, чтобы плейлист открылся уже с картинками
        let artwork: Vec<String> = matched.iter().take(24).filter_map(|t| t.artwork.clone()).collect();
        let _ = bg(async move {
            stream::iter(artwork).map(|u| async move { let _ = svc().media.image_bytes(&u).await; }).buffer_unordered(4).collect::<Vec<_>>().await;
        })
        .await;

        let name = format!("{} · {title}", service.short());
        let id = {
            let mut lib = st.library;
            let mut w = lib.write();
            let id = w.create_playlist(&name);
            if let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) {
                p.tracks = matched;
            }
            id
        };
        let message = format!("«{name}»: найдено {found} из {total} треков");
        toast(st, ToastKind::Success, message.clone());
        put(busy, false);
        put(status, Some((true, message)));
        // пользователь ждёт в окне — сразу открываем плейлист
        if import_dialog_open(st) {
            let mut m = st.modal;
            m.set(None);
            navigate(st, Route::LocalPlaylist(id));
        }
    });
}

fn import_dialog_open(st: AppState) -> bool {
    matches!(*st.modal.peek(), Some(Modal::Import))
}
