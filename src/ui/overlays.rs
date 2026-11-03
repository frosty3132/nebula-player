//! Плавающие слои: очередь, контекстное меню, диалоги, уведомления
//! и подсказка режима редактирования.

use super::actions::*;
use super::icons::Icon;
use super::lyrics_view::LyricsSearch;
use super::player_bar::Cover;
use super::state::*;
use super::widgets::{IconBtn, Toggle};
use crate::model::{fmt_time, ArtSize, UserInfo};
use dioxus::prelude::*;

#[component]
pub fn QueuePanel() -> Element {
    let st = app_state();
    let q = st.queue.read().clone();
    let autoplay = st.settings.read().autoplay_related;
    let cur = q.index.unwrap_or(0);
    let upcoming: Vec<(usize, crate::model::Track)> = q.tracks.iter().cloned().enumerate().skip(cur + 1).take(200).collect();
    let current = q.current().cloned();
    let total_ms: u64 = q.tracks.iter().skip(cur).map(|t| t.duration_ms).sum();
    rsx! {
        aside { class: "queue glass", "data-el": "queue",
            div { class: "q-head",
                div { class: "ly-title", Icon { name: "queue", size: 18 } span { "Очередь" } }
                span { class: "q-meta", "{q.tracks.len()} тр. · {fmt_time(total_ms)}" }
                div { class: "flex1" }
                IconBtn { icon: "delete", size: 17, title: "Очистить предстоящие",
                    onclick: move |_| { let mut qq = st.queue; qq.write().clear_upcoming(); } }
                IconBtn { icon: "close", size: 17, title: "Скрыть (Q)",
                    onclick: move |_| { let mut s = st.settings; s.write().queue_open = false; } }
            }
            if let Some(t) = current {
                div { class: "q-section", "Сейчас" }
                div { class: "q-item now",
                    Cover { src: t.art(ArtSize::Small), class: "q-cover" }
                    div { class: "q-main",
                        div { class: "q-title ellipsis", "{t.title}" }
                        div { class: "q-artist ellipsis", "{t.artist}" }
                    }
                }
            }
            div { class: "q-section", "Далее · {q.label}" }
            div { class: "q-list",
                if upcoming.is_empty() {
                    div { class: "list-empty small", if autoplay { "Дальше — похожие треки (автовоспроизведение)" } else { "Очередь закончится на этом треке" } }
                }
                for (i , t) in upcoming {
                    div {
                        key: "{t.id}-{i}",
                        class: "q-item",
                        ondoubleclick: move |_| jump_to(st, i),
                        oncontextmenu: {
                            let t = t.clone();
                            move |e: MouseEvent| {
                                e.prevent_default();
                                let p = e.client_coordinates();
                                let mut m = st.menu;
                                m.set(Some(MenuState { x: p.x, y: p.y, track: t.clone(), playlist: None, queue_index: Some(i) }));
                            }
                        },
                        Cover { src: t.art(ArtSize::Small), class: "q-cover" }
                        div { class: "q-main", onclick: move |_| jump_to(st, i),
                            div { class: "q-title ellipsis", "{t.title}" }
                            div { class: "q-artist ellipsis", "{t.artist}" }
                        }
                        div { class: "q-tools",
                            IconBtn { icon: "up", size: 15, title: "Выше", onclick: move |_| { let mut qq = st.queue; if i > cur + 1 { qq.write().move_item(i, i - 1); } } }
                            IconBtn { icon: "down", size: 15, title: "Ниже", onclick: move |_| { let mut qq = st.queue; let len = qq.peek().tracks.len(); if i + 1 < len { qq.write().move_item(i, i + 1); } } }
                            IconBtn { icon: "close", size: 15, title: "Убрать", onclick: move |_| { let mut qq = st.queue; qq.write().remove(i); } }
                        }
                    }
                }
            }
            div { class: "q-foot",
                span { Icon { name: "radio", size: 16 } "Похожие после очереди" }
                Toggle { value: autoplay, onchange: move |v| { let mut s = st.settings; s.write().autoplay_related = v; } }
            }
        }
    }
}

#[component]
pub fn ContextMenu() -> Element {
    let st = app_state();
    let Some(menu) = st.menu.read().clone() else { return rsx! {} };
    let t = menu.track.clone();
    let liked = st.library.read().is_favorite(&t.id);
    let playlists: Vec<(String, String)> = st.library.read().playlists.iter().map(|p| (p.id.clone(), p.name.clone())).collect();
    let close = move || {
        let mut m = st.menu;
        m.set(None);
    };
    let (t1, t2, t3, t4, t5, t6, t7, t8) = (t.clone(), t.clone(), t.clone(), t.clone(), t.clone(), t.clone(), t.clone(), t.clone());
    let user = t.uploader_urn.clone().map(|urn| UserInfo { urn, username: t.uploader.clone(), ..Default::default() });
    let is_now = st.now.read().as_ref().is_some_and(|n| n.id == t.id);
    let style = format!("left: min({}px, calc(100vw - 268px)); top: min({}px, calc(100vh - 430px));", menu.x.max(8.0), menu.y.max(8.0));
    rsx! {
        div { class: "ctx-backdrop", onclick: move |_| close(), oncontextmenu: move |e| { e.prevent_default(); close(); } }
        div { class: "ctx-menu glass", "data-el": "menu", style: "{style}",
            div { class: "ctx-head",
                Cover { src: t.art(ArtSize::Small), class: "ctx-cover" }
                div { class: "q-main",
                    div { class: "q-title ellipsis", "{t.title}" }
                    div { class: "q-artist ellipsis", "{t.artist}" }
                }
            }
            button { class: "ctx-item", onclick: move |_| { close(); play_single(st, t1.clone()); }, Icon { name: "play", size: 17 } "Играть" }
            button { class: "ctx-item", onclick: move |_| { close(); add_next(st, t2.clone()); }, Icon { name: "playlist_add", size: 17 } "Играть следующим" }
            button { class: "ctx-item", onclick: move |_| { close(); add_to_queue(st, t3.clone()); }, Icon { name: "queue", size: 17 } "Добавить в очередь" }
            div { class: "ctx-sep" }
            button { class: "ctx-item", onclick: move |_| { close(); toggle_like(st, &t4); },
                Icon { name: if liked { "heart" } else { "heart_outline" }, size: 17 }
                if liked { "Убрать из избранного" } else { "В избранное" }
            }
            button { class: "ctx-item",
                onclick: move |_| {
                    close();
                    let mut m = st.modal;
                    m.set(Some(Modal::AddToPlaylist { track: t5.clone() }));
                },
                Icon { name: "playlist", size: 17 } "В плейлист…"
                if !playlists.is_empty() {
                    span { class: "ctx-hint", "{playlists.len()}" }
                }
            }
            if let Some(pid) = menu.playlist.clone() {
                button { class: "ctx-item danger",
                    onclick: move |_| {
                        close();
                        let mut lib = st.library;
                        let mut w = lib.write();
                        if let Some(p) = w.playlists.iter_mut().find(|p| p.id == pid) {
                            p.tracks.retain(|x| x.id != t6.id);
                        }
                    },
                    Icon { name: "delete", size: 17 } "Убрать из плейлиста"
                }
            }
            if let Some(qi) = menu.queue_index {
                button { class: "ctx-item danger", onclick: move |_| { close(); let mut q = st.queue; q.write().remove(qi); }, Icon { name: "close", size: 17 } "Убрать из очереди" }
            }
            div { class: "ctx-sep" }
            if let Some(u) = user {
                button { class: "ctx-item", onclick: move |_| { close(); navigate(st, Route::User(u.clone())); }, Icon { name: "person", size: 17 } "К исполнителю" }
            }
            if is_now {
                button { class: "ctx-item", onclick: move |_| { close(); let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); }, Icon { name: "mic", size: 17 } "Найти текст…" }
            }
            button { class: "ctx-item", onclick: move |_| { close(); open_on_soundcloud(st, &t7); },
                Icon { name: "external", size: 17 }
                if t8.is_local() { "Показать в папке" } else { "Открыть на SoundCloud" }
            }
        }
    }
}

#[component]
pub fn Toasts() -> Element {
    let st = app_state();
    let list = st.toasts.read().clone();
    rsx! {
        div { class: "toasts",
            for t in list {
                div {
                    key: "{t.id}",
                    class: match t.kind { ToastKind::Error => "toast error", ToastKind::Success => "toast success", ToastKind::Info => "toast" },
                    "data-el": "toast",
                    onclick: move |_| { let mut ts = st.toasts; ts.write().retain(|x| x.id != t.id); },
                    Icon { name: match t.kind { ToastKind::Error => "warning", ToastKind::Success => "check", ToastKind::Info => "info" }, size: 18 }
                    span { "{t.text}" }
                }
            }
        }
    }
}

#[component]
pub fn ModalHost() -> Element {
    let st = app_state();
    let Some(modal) = st.modal.read().clone() else { return rsx! {} };
    let close = move || {
        let mut m = st.modal;
        m.set(None);
    };
    let (title, body) = match modal {
        Modal::NewPlaylist { add } => ("Новый плейлист".to_string(), rsx! { NameForm { initial: String::new(), button: "Создать", on_submit: move |name: String| {
            let mut lib = st.library;
            let id = lib.write().create_playlist(&name);
            if let Some(t) = &add { lib.write().add_to_playlist(&id, t); }
            close();
            toast(st, ToastKind::Success, format!("Плейлист «{name}» создан"));
        } } }),
        Modal::RenamePlaylist { id, name } => ("Переименовать плейлист".to_string(), rsx! { NameForm { initial: name, button: "Сохранить", on_submit: move |name: String| {
            let mut lib = st.library;
            if let Some(p) = lib.write().playlists.iter_mut().find(|p| p.id == id) { p.name = name.clone(); }
            close();
        } } }),
        Modal::AddToPlaylist { track } => {
            let playlists: Vec<(String, String, bool)> = st.library.read().playlists.iter().map(|p| (p.id.clone(), p.name.clone(), p.tracks.iter().any(|x| x.id == track.id))).collect();
            let t_new = track.clone();
            ("Добавить в плейлист".to_string(), rsx! {
                div { class: "pl-pick",
                    for (id , name , has) in playlists {
                        {
                            let t = track.clone();
                            rsx! {
                                button { key: "{id}", class: "lyr-item", disabled: has,
                                    onclick: move |_| {
                                        let mut lib = st.library;
                                        lib.write().add_to_playlist(&id, &t);
                                        close();
                                        toast(st, ToastKind::Success, format!("Добавлено в «{name}»"));
                                    },
                                    Icon { name: "playlist", size: 18 }
                                    span { class: "lyr-main ellipsis", "{name}" }
                                    if has { span { class: "badge soft", "уже есть" } }
                                }
                            }
                        }
                    }
                    button { class: "btn primary", onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::NewPlaylist { add: Some(t_new.clone()) })); },
                        Icon { name: "add", size: 18 } "Новый плейлист"
                    }
                }
            })
        }
        Modal::LyricsSearch => ("Поиск текста в LRCLIB".to_string(), rsx! { LyricsSearch {} }),
        Modal::Confirm { title, text, action } => (title, rsx! {
            p { class: "modal-text", "{text}" }
            div { class: "modal-actions",
                button { class: "btn ghost", onclick: move |_| close(), "Отмена" }
                button { class: "btn danger",
                    onclick: move |_| {
                        match &action {
                            ConfirmAction::DeletePlaylist(id) => {
                                let mut lib = st.library;
                                lib.write().playlists.retain(|p| &p.id != id);
                                navigate(st, Route::Playlists);
                            }
                            ConfirmAction::ClearCache => {
                                crate::media::clear_cache();
                                toast(st, ToastKind::Success, "Кэш очищен");
                            }
                            ConfirmAction::ResetSettings => {
                                let mut s = st.settings;
                                let keep_folders = s.peek().music_folders.clone();
                                s.set(crate::settings::Settings { music_folders: keep_folders, ..Default::default() });
                                toast(st, ToastKind::Success, "Настройки сброшены");
                            }
                            ConfirmAction::DeleteTheme(id) => {
                                crate::theme::reset_or_delete(id);
                                let mut themes = st.themes;
                                themes.set(crate::theme::load_all());
                                if st.settings.peek().theme == *id && !crate::theme::is_builtin(id) {
                                    let mut s = st.settings;
                                    s.write().theme = "galaxy".into();
                                }
                                let current = st.settings.peek().theme.clone();
                                let found = themes.peek().iter().find(|t| t.id == current).cloned();
                                if let Some(t) = found {
                                    let mut th = st.theme;
                                    th.set(t);
                                }
                            }
                        }
                        close();
                    },
                    "Подтвердить"
                }
            }
        }),
    };
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| close(),
            div { class: "modal glass", "data-el": "modal", onclick: move |e| e.stop_propagation(),
                div { class: "modal-head",
                    h3 { "{title}" }
                    IconBtn { icon: "close", size: 18, title: "Закрыть", onclick: move |_| close() }
                }
                {body}
            }
        }
    }
}

#[component]
fn NameForm(initial: String, button: String, on_submit: EventHandler<String>) -> Element {
    let mut name = use_signal(|| initial.clone());
    let submit = move || {
        let n = name.peek().trim().to_string();
        if !n.is_empty() {
            on_submit.call(n);
        }
    };
    rsx! {
        input {
            class: "input big",
            "data-el": "input",
            autofocus: true,
            placeholder: "Название",
            value: "{name}",
            oninput: move |e: FormEvent| name.set(e.value()),
            onkeydown: move |e: KeyboardEvent| if e.key() == Key::Enter { submit() },
        }
        div { class: "modal-actions",
            button { class: "btn primary", onclick: move |_| submit(), "{button}" }
        }
    }
}

#[component]
pub fn EditHint() -> Element {
    let st = app_state();
    rsx! {
        div { class: "edit-hint glass",
            Icon { name: "brush", size: 18 }
            span { "Режим редактирования: наведите и кликните по любому элементу" }
            span { id: "edit-hover", class: "edit-hover" }
            button { class: "btn small", onclick: move |_| { let mut e = st.edit_mode; e.set(false); let mut s = st.selected_el; s.set(None); }, "Готово (Esc)" }
        }
    }
}
