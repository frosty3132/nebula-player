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
                    span { class: "ic-sub", "Свои плейлисты по ссылке · нужен вход в Spotify" }
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
    // Яндекс: необязательный OAuth-токен; Spotify: нужен привязанный аккаунт
    let mut token = use_signal(|| svc().yandex.token());
    let mut show_token = use_signal(|| !svc().yandex.token().is_empty());
    let sp_linked = svc().spotify.user().is_some();

    let run = move || {
        let input = url.peek().trim().to_string();
        if input.is_empty() || *busy.peek() {
            return;
        }
        start_import(st, service, input, busy, status);
    };

    let busy_now = *busy.read();
    let (placeholder, hint) = match service {
        Service::Yandex => ("https://music.yandex.ru/users/…/playlists/…", "Ссылка вида music.yandex.ru/users/…/playlists/… или music.yandex.ru/playlists/… Плейлист должен быть публичным."),
        Service::Spotify => ("https://open.spotify.com/playlist/…", "Ссылка на ваш собственный или совместный плейлист (в Spotify: «Поделиться» → «Скопировать ссылку»). Все плейлисты и любимые треки сразу — в Настройки → SoundCloud → Spotify."),
    };
    rsx! {
        div { class: "import-form",
            div { class: "import-back",
                IconBtn { icon: "back", title: "К выбору сервиса", size: 18, onclick: move |_| if !*busy.peek() { picker.set(None) } }
                span { class: "import-service", "{service.label()}" }
            }
            if service == Service::Spotify && !sp_linked {
                div { class: "import-keys",
                    p { class: "modal-text", "Spotify отдаёт треки только из ваших плейлистов и только после входа в аккаунт. Привяжите Spotify в настройках — там же можно перенести все плейлисты и любимые треки разом." }
                    div { class: "import-row end",
                        button { class: "btn primary",
                            onclick: move |_| {
                                let mut m = st.modal;
                                m.set(None);
                                navigate(st, Route::Settings(SettingsTab::Account));
                            },
                            Icon { name: "login", size: 18 } "Привязать Spotify"
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
                    Service::Spotify => rsx! {},
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
pub fn start_import(st: AppState, service: Service, input: String, busy: Signal<bool>, status: Signal<Option<(bool, String)>>) {
    spawn_forever(async move {
        run_import(st, service, input, busy, status).await;
    });
}

/// То же, но можно дождаться: true — плейлист создан. Так «Перенести все»
/// импортирует плейлисты по очереди, не нагружая поиск SoundCloud.
pub async fn run_import(st: AppState, service: Service, input: String, busy: Signal<bool>, status: Signal<Option<(bool, String)>>) -> bool {
    put(busy, true);
    put(status, Some((true, format!("Загружаю плейлист из {}…", service.label()))));
    let local = st.local.peek().clone();
    let sc_ready = *st.sc_ready.peek();
    {
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
                let e = if service == Service::Yandex && e.contains("451") {
                    // проверено: без токена API Яндекса отвечает 451 на любой запрос вне РФ/СНГ
                    "Яндекс Музыка не отдаёт плейлисты в вашем регионе (ответ 451): её API работает только из России и стран СНГ. Укажите OAuth-токен своего аккаунта Яндекса — с ним запросы обычно проходят — или подключитесь через российский IP.".to_string()
                } else if service == Service::Yandex && e.contains("401") {
                    "Яндекс не принял OAuth-токен (ответ 401): он устарел или скопирован не целиком. Получите новый и сохраните его заново.".to_string()
                } else {
                    e
                };
                put(status, Some((false, e.clone())));
                put(busy, false);
                if !import_dialog_open(st) {
                    toast(st, ToastKind::Error, format!("Импорт из {}: {e}", service.label()));
                }
                return false;
            }
        };
        let total = playlist.tracks.len();
        let title = playlist.title.clone();
        let search_note = if sc_ready { "в вашей музыке и на SoundCloud" } else { "в вашей музыке" };
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
    }
    true
}

fn import_dialog_open(st: AppState) -> bool {
    matches!(*st.modal.peek(), Some(Modal::Import))
}
