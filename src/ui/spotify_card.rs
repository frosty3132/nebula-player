//! Карточка «Spotify» во вкладке SoundCloud: привязка аккаунта и перенос
//! плейлистов и любимых треков (см. `crate::spotify`). Треки ищутся в вашей
//! музыке и на SoundCloud; найденные любимые попадают в «Избранное».

use super::actions::*;
use super::icons::Icon;
use super::import_dialog::run_import;
use super::state::*;
use super::widgets::*;
use crate::import::{self, Service};
use crate::services::svc;
use crate::spotify::{SpPlaylist, DASHBOARD_URL, REDIRECT_URI};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

/// Сколько ненайденных любимых треков искать на SoundCloud за один перенос.
const LIKED_SC_SEARCH: usize = 400;

/// Записать в сигнал, если карточка ещё на экране.
fn put<T: 'static>(mut sig: Signal<T>, value: T) {
    if let Ok(mut w) = sig.try_write() {
        *w = value;
    }
}

#[component]
pub fn SpotifyCard() -> Element {
    // версия состояния привязки: меняется при входе/выходе, чтобы карточка перерисовалась
    let mut rev = use_signal(|| 0u32);
    let _ = rev();
    let user = svc().spotify.user();
    let has_id = svc().spotify.has_client_id();
    rsx! {
        Card { title: "Spotify", icon: "radio", desc: "Привязка аккаунта и перенос плейлистов и любимых треков",
            match (user, has_id) {
                (Some(u), _) => rsx! { Linked { name: u.name, avatar: u.avatar, onlogout: move |_| { svc().spotify.logout(); rev += 1; } } },
                (None, true) => rsx! { LinkButton { onlinked: move |_| rev += 1, onreset: move |_| { svc().spotify.set_client_id(""); rev += 1; } } },
                (None, false) => rsx! { Setup { onsaved: move |_| rev += 1 } },
            }
        }
    }
}

/// Первый шаг: Client ID своего приложения Spotify.
#[component]
fn Setup(onsaved: EventHandler<()>) -> Element {
    let mut id = use_signal(String::new);
    rsx! {
        p { class: "set-text",
            "Spotify пускает к плейлистам и любимым трекам только через ваше собственное приложение — это бесплатно и делается один раз:"
        }
        ol { class: "set-list sp-steps",
            li {
                "Откройте "
                a { class: "link", onclick: move |_| { let _ = open::that(DASHBOARD_URL); }, "developer.spotify.com/dashboard" }
                " → Create app. Название любое."
            }
            li { "Redirect URI: " code { class: "sp-code", "{REDIRECT_URI}" } ", API — Web API. Сохраните." }
            li { "Скопируйте Client ID из настроек приложения и вставьте ниже. Секретный ключ не нужен." }
        }
        p { class: "set-hint", "Spotify требует Premium у владельца приложения. Других людей (до 5) можно добавить в Dashboard → User Management." }
        div { class: "sp-id-row",
            input { class: "input", r#type: "text", autocomplete: "off", spellcheck: false, placeholder: "Client ID",
                value: "{id}", oninput: move |e: FormEvent| id.set(e.value()) }
            button { class: "btn", disabled: id.read().trim().len() < 16,
                onclick: move |_| {
                    svc().spotify.set_client_id(&id.peek());
                    onsaved.call(());
                },
                Icon { name: "save", size: 18 } "Сохранить"
            }
        }
    }
}

/// Второй шаг: вход в аккаунт Spotify в браузере.
#[component]
fn LinkButton(onlinked: EventHandler<()>, onreset: EventHandler<()>) -> Element {
    let mut busy = use_signal(|| false);
    let mut status = use_signal(|| None::<(bool, String)>);
    let link = move |_| {
        busy.set(true);
        status.set(Some((true, "Подтвердите вход на странице Spotify в браузере…".into())));
        spawn(async move {
            let res = bg(async { svc().spotify.login().await }).await;
            busy.set(false);
            match res {
                Some(Ok(_)) => {
                    status.set(None);
                    onlinked.call(());
                }
                Some(Err(e)) => status.set(Some((false, e))),
                None => status.set(Some((false, "Внутренняя ошибка".into()))),
            }
        });
    };
    rsx! {
        p { class: "set-text", "Войдите в Spotify — Nebula получит доступ только на чтение: ваши плейлисты и любимые треки." }
        div { class: "set-actions",
            button { class: "btn", disabled: busy(), onclick: link, Icon { name: "login", size: 18 } "Привязать Spotify" }
            if busy() {
                Spinner {}
            }
            button { class: "link-btn", disabled: busy(), onclick: move |_| onreset.call(()), "Изменить Client ID" }
        }
        if let Some((ok, text)) = status.read().clone() {
            div { class: if ok { "note" } else { "note err" }, "{text}" }
        }
    }
}

/// Аккаунт привязан: любимые треки и плейлисты.
#[component]
fn Linked(name: String, avatar: Option<String>, onlogout: EventHandler<()>) -> Element {
    let st = app_state();
    let playlists = use_resource(|| async move { bg(async { svc().spotify.my_playlists().await }).await.unwrap_or_else(|| Err("Внутренняя ошибка".into())) });
    let liked_busy = use_signal(|| false);
    let liked_status = use_signal(|| None::<(bool, String)>);
    let all_busy = use_signal(|| false);
    let all_status = use_signal(|| None::<(bool, String)>);
    let avatar = avatar.map(|a| image_url_raw(&a));
    let list = playlists.read().clone();
    let own: Vec<SpPlaylist> = match &list {
        Some(Ok(v)) => v.iter().filter(|p| p.own).cloned().collect(),
        _ => Vec::new(),
    };
    let own_ids: Vec<String> = own.iter().map(|p| p.id.clone()).collect();
    rsx! {
        div { class: "sp-profile",
            div { class: "sp-avatar",
                match avatar {
                    Some(a) => rsx! { img { src: "{a}", alt: "", draggable: false } },
                    None => rsx! { Icon { name: "person", size: 20 } },
                }
            }
            div { class: "sp-who",
                div { class: "sp-name ellipsis", "{name}" }
                div { class: "set-hint", "Spotify привязан" }
            }
            button { class: "btn ghost", onclick: move |_| onlogout.call(()), Icon { name: "logout", size: 18 } "Отвязать" }
        }
        div { class: "sp-block",
            div { class: "sp-row",
                span { class: "sp-liked-icon", Icon { name: "heart", size: 18 } }
                div { class: "sp-who",
                    div { class: "sp-name", "Любимые треки" }
                    div { class: "set-hint", "Найденные треки попадут в «Избранное», все — в плейлист «Spotify · Любимые треки»" }
                }
                button { class: "btn", disabled: liked_busy(), onclick: move |_| import_liked(st, liked_busy, liked_status),
                    Icon { name: "download", size: 18 } "Перенести"
                }
            }
            if let Some((ok, text)) = liked_status.read().clone() {
                div { class: if ok { "note ok" } else { "note err" }, "{text}" }
            }
        }
        div { class: "sp-block",
            div { class: "sp-head",
                span { "Плейлисты" }
                if own.len() > 1 {
                    button { class: "link-btn", disabled: all_busy(),
                        onclick: move |_| import_all(st, own_ids.clone(), all_busy, all_status),
                        "Перенести все свои ({own.len()})"
                    }
                }
            }
            if let Some((ok, text)) = all_status.read().clone() {
                div { class: if ok { "note ok" } else { "note err" }, "{text}" }
            }
            match list {
                None => rsx! { Spinner {} },
                Some(Err(e)) => rsx! { div { class: "note err", "{e}" } },
                Some(Ok(v)) if v.is_empty() => rsx! { p { class: "set-hint", "В медиатеке Spotify нет плейлистов" } },
                Some(Ok(v)) => rsx! {
                    for p in v {
                        PlaylistRow { key: "{p.id}", p }
                    }
                },
            }
        }
    }
}

#[component]
fn PlaylistRow(p: SpPlaylist) -> Element {
    let st = app_state();
    let busy = use_signal(|| false);
    let status = use_signal(|| None::<(bool, String)>);
    let id = p.id.clone();
    let image = p.image.as_deref().map(image_url_raw);
    rsx! {
        div { class: "sp-row",
            div { class: "sp-cover",
                match image {
                    Some(src) => rsx! { img { src: "{src}", alt: "", draggable: false, loading: "lazy" } },
                    None => rsx! { Icon { name: "playlist", size: 18 } },
                }
            }
            div { class: "sp-who",
                div { class: "sp-name ellipsis", "{p.name}" }
                div { class: "set-hint ellipsis",
                    if p.own { "{p.total} треков" } else { "Чужой плейлист — Spotify отдаёт треки только из своих" }
                    if let Some((ok, text)) = status.read().clone() {
                        span { class: if ok { "sp-ok" } else { "sp-err" }, " · {text}" }
                    }
                }
            }
            if p.own {
                button { class: "btn ghost", disabled: busy(),
                    onclick: move |_| {
                        let id = id.clone();
                        spawn_forever(async move {
                            run_import(st, Service::Spotify, id, busy, status).await;
                        });
                    },
                    if busy() { "Переношу…" } else { "Перенести" }
                }
            }
        }
    }
}

/// Обложки Spotify — внешние https-картинки, через кэш `/media` как остальные.
fn image_url_raw(url: &str) -> String {
    crate::model::image_url(url)
}

/// Перенести все свои плейлисты по очереди.
fn import_all(st: AppState, ids: Vec<String>, busy: Signal<bool>, status: Signal<Option<(bool, String)>>) {
    put(busy, true);
    let total = ids.len();
    spawn_forever(async move {
        let mut ok = 0;
        for (i, id) in ids.into_iter().enumerate() {
            put(status, Some((true, format!("Переношу плейлист {} из {total}…", i + 1))));
            // у каждого — свои сигналы, чтобы общий статус не перетирался сообщениями импорта
            let row_busy = Signal::new(false);
            let row_status = Signal::new(None);
            if run_import(st, Service::Spotify, id, row_busy, row_status).await {
                ok += 1;
            }
        }
        put(busy, false);
        put(status, Some((ok == total, format!("Перенесено плейлистов: {ok} из {total}"))));
    });
}

/// Любимые треки: найденные — в «Избранное» (новые сверху), все вместе с
/// ненайденными — в плейлист «Spotify · Любимые треки».
fn import_liked(st: AppState, busy: Signal<bool>, status: Signal<Option<(bool, String)>>) {
    put(busy, true);
    put(status, Some((true, "Загружаю любимые треки из Spotify…".into())));
    let local = st.local.peek().clone();
    let sc_ready = *st.sc_ready.peek();
    spawn_forever(async move {
        let fetched = bg(async { svc().spotify.liked().await }).await.unwrap_or_else(|| Err("Внутренняя ошибка".into()));
        let liked = match fetched {
            Ok(l) => l,
            Err(e) => {
                put(busy, false);
                put(status, Some((false, e)));
                return;
            }
        };
        let total = liked.tracks.len();
        let where_ = if sc_ready { "в вашей музыке и на SoundCloud" } else { "в вашей музыке (войдите в SoundCloud, чтобы искать и там)" };
        put(status, Some((true, format!("Ищу {total} треков {where_}…"))));
        let matched = bg(async move {
            let sc = if sc_ready { Some(&svc().sc) } else { None };
            import::match_tracks_with(&liked, &local, sc, LIKED_SC_SEARCH).await
        })
        .await
        .unwrap_or_default();
        let found: Vec<_> = matched.iter().filter(|t| import::is_matched(t)).cloned().collect();
        let added = {
            let mut lib = st.library;
            let mut w = lib.write();
            // список Spotify — новые сверху; вставляем с конца, чтобы порядок сохранился
            let mut added = 0;
            for t in found.iter().rev() {
                if !w.is_favorite(&t.id) {
                    w.favorites.insert(0, t.clone());
                    added += 1;
                }
            }
            let id = w.create_playlist("Spotify · Любимые треки");
            if let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) {
                p.tracks = matched;
            }
            added
        };
        let msg = format!("Найдено {} из {total}; в «Избранное» добавлено {added}. Все треки — в плейлисте «Spotify · Любимые треки»", found.len());
        toast(st, ToastKind::Success, msg.clone());
        put(busy, false);
        put(status, Some((true, msg)));
    });
}
