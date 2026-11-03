//! Страницы: главная, поиск, жанры, избранное, история, плейлисты,
//! исполнитель, локальная музыка и личный SoundCloud.

use super::actions::*;
use super::icons::Icon;
use super::player_bar::Cover;
use super::state::*;
use super::tracks::*;
use super::widgets::{Empty, IconBtn, Spinner};
use crate::model::{fmt_count, image_url, sc_art_size, ArtSize, Page, PlaylistInfo, Track, UserInfo};
use crate::services::svc;
use crate::theme::HOME_SECTIONS;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::time::Duration;

/// (жанр в API, подпись, цвет 1, цвет 2, символ)
pub const GENRES: &[(&str, &str, &str, &str, &str)] = &[
    ("Phonk", "Фонк", "#7c3aed", "#db2777", "👻"),
    ("Hip-hop & Rap", "Хип-хоп и рэп", "#f59e0b", "#ef4444", "🎤"),
    ("Electronic", "Электроника", "#06b6d4", "#3b82f6", "⚡"),
    ("House", "Хаус", "#22c55e", "#14b8a6", "🏠"),
    ("Techno", "Техно", "#64748b", "#0ea5e9", "🛸"),
    ("Drum & Bass", "Drum & Bass", "#f97316", "#a855f7", "🥁"),
    ("Lo-fi", "Lo-Fi", "#a78bfa", "#f472b6", "☕"),
    ("Ambient", "Эмбиент", "#0ea5e9", "#6366f1", "🌌"),
    ("Synthwave", "Синтвейв", "#ff2a6d", "#05d9e8", "🌆"),
    ("Trap", "Трэп", "#e11d48", "#7c3aed", "💎"),
    ("Pop", "Поп", "#ec4899", "#f59e0b", "🎧"),
    ("Rock", "Рок", "#ef4444", "#78716c", "🎸"),
    ("Metal", "Метал", "#525252", "#dc2626", "🤘"),
    ("R&B & Soul", "R&B и соул", "#8b5cf6", "#ec4899", "💜"),
    ("Jazz & Blues", "Джаз и блюз", "#d97706", "#7c2d12", "🎷"),
    ("Classical", "Классика", "#94a3b8", "#6366f1", "🎻"),
    ("Dubstep", "Дабстеп", "#84cc16", "#06b6d4", "🔊"),
    ("Deep House", "Дип-хаус", "#0891b2", "#7c3aed", "🌊"),
    ("Dance & EDM", "Dance и EDM", "#22d3ee", "#e879f9", "💃"),
    ("Trance", "Транс", "#3b82f6", "#a855f7", "✨"),
    ("Indie", "Инди", "#f472b6", "#facc15", "🌻"),
    ("Chill", "Чилл", "#14b8a6", "#a78bfa", "🌙"),
    ("Soundtrack", "Саундтреки", "#6366f1", "#1e1b4b", "🎬"),
    ("Reggaeton", "Реггетон", "#f59e0b", "#22c55e", "🌴"),
];

fn genre_label(api: &str) -> String {
    GENRES.iter().find(|g| g.0 == api).map(|g| g.1.to_string()).unwrap_or_else(|| api.to_string())
}

#[component]
pub fn PageHeader(
    title: String,
    #[props(default)] kicker: String,
    #[props(default)] subtitle: String,
    #[props(default)] art: Option<String>,
    #[props(default)] round: bool,
    #[props(default)] gradient: Option<(String, String)>,
    children: Element,
) -> Element {
    let style = gradient.map(|(a, b)| format!("--g1: {a}; --g2: {b};")).unwrap_or_default();
    rsx! {
        div { class: "page-head", style: "{style}",
            if let Some(a) = art {
                Cover { src: Some(a), class: if round { "ph-art round" } else { "ph-art" } }
            }
            div { class: "ph-text",
                if !kicker.is_empty() {
                    div { class: "ph-kicker", "{kicker}" }
                }
                h1 { class: "page-title", "data-el": "page_title", "{title}" }
                if !subtitle.is_empty() {
                    div { class: "ph-sub", "{subtitle}" }
                }
                div { class: "ph-actions", {children} }
            }
        }
    }
}

#[component]
fn PlayButtons(tracks: Vec<Track>, label: String) -> Element {
    let st = app_state();
    let (t1, t2, l1, l2) = (tracks.clone(), tracks.clone(), label.clone(), label.clone());
    let empty = tracks.is_empty();
    rsx! {
        button { class: "btn primary", "data-el": "button", disabled: empty,
            onclick: move |_| play_list(st, t1.clone(), 0, &l1),
            Icon { name: "play", size: 18 } "Слушать"
        }
        button { class: "btn ghost", "data-el": "button", disabled: empty,
            onclick: move |_| {
                let mut v = t2.clone();
                shuffle_in_place(&mut v);
                play_list(st, v, 0, &l2);
            },
            Icon { name: "shuffle", size: 18 } "Вперемешку"
        }
    }
}

#[component]
fn Section(title: String, #[props(default)] action: Option<(String, Route)>, children: Element) -> Element {
    let st = app_state();
    rsx! {
        section { class: "section",
            div { class: "section-head",
                h2 { class: "section-title", "data-el": "section_title", "{title}" }
                if let Some((label, route)) = action {
                    button { class: "link-btn", onclick: move |_| navigate(st, route.clone()), "{label}" }
                }
            }
            {children}
        }
    }
}

#[component]
fn ErrorBox(text: String) -> Element {
    let st = app_state();
    let needs_setup = text.contains("не подключён");
    rsx! {
        div { class: "error-box glass",
            Icon { name: "warning", size: 20 }
            span { "{text}" }
            if needs_setup {
                button { class: "btn small", onclick: move |_| navigate(st, Route::Settings(SettingsTab::Account)), "Открыть настройки" }
            }
        }
    }
}

// ───────────────────────────────────── главная ─────────────────────────────────────

#[component]
pub fn Home() -> Element {
    let st = app_state();
    let sections = st.theme.read().layout.home.clone();
    rsx! {
        div { class: "page home",
            for entry in sections.iter().filter(|e| e.visible) {
                match entry.id.as_str() {
                    "hero" => rsx! { Hero { key: "{entry.id}" } },
                    "continue" => rsx! { ContinueCard { key: "{entry.id}" } },
                    "recent" => rsx! { RecentSection { key: "{entry.id}" } },
                    "related" => rsx! { RelatedSection { key: "{entry.id}" } },
                    "genres" => rsx! {
                        Section { key: "{entry.id}", title: "Жанры и настроения", action: Some(("Все жанры".to_string(), Route::Genres)),
                            GenreGrid { limit: 8 }
                        }
                    },
                    "favorites" => rsx! { FavoritesSection { key: "{entry.id}" } },
                    "local" => rsx! { LocalSection { key: "{entry.id}" } },
                    _ => rsx! {},
                }
            }
            if sections.iter().all(|e| !e.visible) {
                Empty { icon: "home", title: "Главная пуста", text: "Все разделы скрыты в настройках расположения." }
            }
            div { class: "home-foot", "Nebula · данные SoundCloud и LRCLIB" }
            // упоминание HOME_SECTIONS для редактора раскладки
            span { class: "hidden", "{HOME_SECTIONS.len()}" }
        }
    }
}

#[component]
fn Hero() -> Element {
    let st = app_state();
    let hour = use_resource(|| async move { document::eval("return new Date().getHours();").join::<u32>().await.unwrap_or(12) });
    let greeting = match hour.read().as_ref().copied().unwrap_or(12) {
        5..=11 => "Доброе утро",
        12..=17 => "Добрый день",
        18..=22 => "Добрый вечер",
        _ => "Доброй ночи",
    };
    let sc_ready = *st.sc_ready.read();
    let mut q = use_signal(String::new);
    rsx! {
        section { class: "hero glass", "data-el": "hero",
            div { class: "hero-orbit", span {} span {} span {} }
            div { class: "hero-text",
                div { class: "ph-kicker", "✦ Nebula" }
                h1 { class: "hero-title", "{greeting}, путешественник" }
                p { class: "hero-sub", "Музыка из SoundCloud и с вашего компьютера, синхронные тексты из LRCLIB и интерфейс, который меняется как вам угодно." }
                div { class: "hero-search", "data-el": "search",
                    Icon { name: "search", size: 20 }
                    input {
                        class: "hero-input",
                        placeholder: "Найти трек, исполнителя или плейлист…",
                        value: "{q}",
                        oninput: move |e: FormEvent| q.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter && !q.peek().trim().is_empty() {
                                let mut sq = st.search_query;
                                sq.set(q.peek().clone());
                                navigate(st, Route::Search);
                            }
                        },
                    }
                }
                if !sc_ready {
                    div { class: "hero-note",
                        Icon { name: "cloud", size: 18 }
                        span { "SoundCloud ещё не подключён — пока доступна локальная музыка." }
                        button { class: "btn small primary", onclick: move |_| navigate(st, Route::Settings(SettingsTab::Account)), "Подключить" }
                    }
                }
            }
        }
    }
}

#[component]
fn ContinueCard() -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let Some(t) = now else { return rsx! {} };
    let label = st.queue.read().label.clone();
    let art = t.art(ArtSize::Medium);
    let playing = st.pstate.read().is_active();
    rsx! {
        section { class: "continue glass",
            Cover { src: art, class: "cont-art" }
            div { class: "cont-text",
                div { class: "ph-kicker", if playing { "Сейчас играет" } else { "Продолжить" } }
                div { class: "cont-title ellipsis", "{t.title}" }
                div { class: "cont-artist ellipsis", "{t.artist} · {label}" }
            }
            button { class: "play-btn big", "data-el": "play_button", onclick: move |_| toggle_play(st),
                Icon { name: if playing { "pause" } else { "play" }, size: 26 }
            }
        }
    }
}

#[component]
fn RecentSection() -> Element {
    let st = app_state();
    let items: Vec<Track> = st.library.read().history.iter().take(18).cloned().collect();
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        Section { title: "Недавно слушали", action: Some(("История".to_string(), Route::History)),
            CardRow { tracks: items, label: "История" }
        }
    }
}

#[component]
fn FavoritesSection() -> Element {
    let st = app_state();
    let items: Vec<Track> = st.library.read().favorites.iter().take(18).cloned().collect();
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        Section { title: "Избранное", action: Some(("Все".to_string(), Route::Favorites)),
            CardRow { tracks: items, label: "Избранное" }
        }
    }
}

#[component]
fn LocalSection() -> Element {
    let st = app_state();
    let items: Vec<Track> = st.local.read().iter().take(18).cloned().collect();
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        Section { title: "Моя музыка", action: Some(("Открыть".to_string(), Route::Local)),
            CardRow { tracks: items, label: "Моя музыка" }
        }
    }
}

#[component]
fn RelatedSection() -> Element {
    let st = app_state();
    let seed = st.library.read().history.iter().find(|t| t.urn.is_some()).cloned();
    let sc_ready = *st.sc_ready.read();
    let seed_id = seed.as_ref().map(|t| t.id.clone());
    // похожие треки кэшируются в состоянии, пока не сменится «затравка»
    use_effect(move || {
        let Some(seed) = st.library.read().history.iter().find(|t| t.urn.is_some()).cloned() else { return };
        if !*st.sc_ready.read() {
            return;
        }
        if st.related.peek().as_ref().is_some_and(|(id, _)| *id == seed.id) {
            return;
        }
        let mut rel = st.related;
        rel.set(Some((seed.id.clone(), Vec::new())));
        let urn = seed.urn.clone().unwrap_or_default();
        spawn(async move {
            if let Some(Ok(p)) = bg(async move { svc().sc.related(&urn).await }).await {
                rel.set(Some((seed.id.clone(), p.items)));
            }
        });
    });
    if !sc_ready {
        return rsx! {};
    }
    let Some(seed) = seed else { return rsx! {} };
    let items = st.related.read().as_ref().filter(|(id, _)| Some(id) == seed_id.as_ref()).map(|(_, v)| v.clone()).unwrap_or_default();
    rsx! {
        Section { title: "Похоже на «{seed.title}»",
            if items.is_empty() {
                div { class: "card-row loading", Spinner {} }
            } else {
                CardRow { tracks: items, label: "Похожие треки" }
            }
        }
    }
}

// ───────────────────────────────────── поиск ─────────────────────────────────────

#[derive(Clone, PartialEq)]
enum Results {
    Tracks(Page<Track>),
    Playlists(Page<PlaylistInfo>),
    Users(Page<UserInfo>),
}

#[component]
pub fn Search() -> Element {
    let st = app_state();
    let mut query = st.search_query;
    let mut tab = use_signal(|| if *st.sc_ready.peek() { "tracks" } else { "local" });
    let mut results = use_signal(|| None::<Results>);
    let mut loading = use_signal(|| false);
    let mut more_loading = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);

    use_effect(move || {
        let q = query.read().trim().to_string();
        let t = *tab.read();
        results.set(None);
        error.set(None);
        if q.is_empty() || t == "local" {
            loading.set(false);
            return;
        }
        loading.set(true);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(380)).await;
            if query.peek().trim() != q || *tab.peek() != t {
                return;
            }
            let q2 = q.clone();
            let res = bg(async move {
                match t {
                    "playlists" => svc().sc.search_playlists(&q2).await.map(Results::Playlists),
                    "users" => svc().sc.search_users(&q2).await.map(Results::Users),
                    _ => svc().sc.search_tracks(&q2).await.map(Results::Tracks),
                }
            })
            .await;
            if query.peek().trim() != q || *tab.peek() != t {
                return;
            }
            match res {
                Some(Ok(r)) => {
                    results.set(Some(r));
                    let mut lib = st.library;
                    lib.write().push_search(&q);
                }
                Some(Err(e)) => error.set(Some(e)),
                None => {}
            }
            loading.set(false);
        });
    });

    let load_more = move |_: ()| {
        let next = match &*results.peek() {
            Some(Results::Tracks(p)) => p.next.clone(),
            Some(Results::Playlists(p)) => p.next.clone(),
            Some(Results::Users(p)) => p.next.clone(),
            None => None,
        };
        let Some(next) = next else { return };
        more_loading.set(true);
        spawn(async move {
            let kind = *tab.peek();
            let n2 = next.clone();
            let res = bg(async move {
                match kind {
                    "playlists" => svc().sc.next_playlists(&n2).await.map(Results::Playlists),
                    "users" => svc().sc.next_users(&n2).await.map(Results::Users),
                    _ => svc().sc.next_tracks(&n2).await.map(Results::Tracks),
                }
            })
            .await;
            if let Some(Ok(page)) = res {
                let mut w = results.write();
                match (w.as_mut(), page) {
                    (Some(Results::Tracks(a)), Results::Tracks(b)) => {
                        a.items.extend(b.items);
                        a.next = b.next;
                    }
                    (Some(Results::Playlists(a)), Results::Playlists(b)) => {
                        a.items.extend(b.items);
                        a.next = b.next;
                    }
                    (Some(Results::Users(a)), Results::Users(b)) => {
                        a.items.extend(b.items);
                        a.next = b.next;
                    }
                    _ => {}
                }
            }
            more_loading.set(false);
        });
    };

    let q_now = query.read().clone();
    let recent = st.library.read().recent_searches.clone();
    let local_hits: Vec<Track> = if *tab.read() == "local" && !q_now.trim().is_empty() {
        let ql = q_now.to_lowercase();
        st.local
            .read()
            .iter()
            .filter(|t| t.title.to_lowercase().contains(&ql) || t.artist.to_lowercase().contains(&ql) || t.album.as_deref().unwrap_or("").to_lowercase().contains(&ql))
            .take(300)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };
    let tabs = [("tracks", "Треки"), ("playlists", "Плейлисты"), ("users", "Исполнители"), ("local", "Моя музыка")];

    rsx! {
        div { class: "page search",
            div { class: "search-bar glass", "data-el": "search",
                Icon { name: "search", size: 22 }
                input {
                    id: "search-input",
                    class: "search-input",
                    autofocus: true,
                    placeholder: "Что будем слушать?",
                    value: "{q_now}",
                    oninput: move |e: FormEvent| query.set(e.value()),
                }
                if !q_now.is_empty() {
                    IconBtn { icon: "close", size: 18, title: "Очистить", onclick: move |_| query.set(String::new()) }
                }
            }
            div { class: "tabs",
                for (id , label) in tabs {
                    button {
                        key: "{id}",
                        class: if *tab.read() == id { "chip active" } else { "chip" },
                        "data-el": "chip",
                        onclick: move |_| tab.set(id),
                        "{label}"
                    }
                }
            }
            if q_now.trim().is_empty() {
                if !recent.is_empty() {
                    Section { title: "Вы искали",
                        div { class: "chips",
                            for r in recent {
                                button { class: "chip", "data-el": "chip", onclick: move |_| query.set(r.clone()), "{r}" }
                            }
                        }
                    }
                }
                Section { title: "Жанры", GenreGrid { limit: 12 } }
            } else if *tab.read() == "local" {
                if local_hits.is_empty() {
                    Empty { icon: "folder", title: "Ничего не найдено", text: "В локальной библиотеке нет совпадений." }
                } else {
                    TrackList { tracks: local_hits, label: "Поиск: моя музыка" }
                }
            } else if let Some(e) = error.read().clone() {
                ErrorBox { text: e }
            } else if *loading.read() && results.read().is_none() {
                SkeletonRows {}
            } else {
                match results.read().clone() {
                    Some(Results::Tracks(p)) => rsx! {
                        if p.items.is_empty() {
                            Empty { icon: "search", title: "Ничего не найдено", text: "Попробуйте другой запрос." }
                        } else {
                            TrackList { tracks: p.items.clone(), label: format!("Поиск: {q_now}"), loading_more: *more_loading.read(),
                                on_more: if p.next.is_some() { Some(EventHandler::new(load_more)) } else { None } }
                        }
                    },
                    Some(Results::Playlists(p)) => rsx! {
                        div { class: "card-grid",
                            for pl in p.items.iter() {
                                PlaylistCard { key: "{pl.id}", info: pl.clone() }
                            }
                        }
                        if p.next.is_some() {
                            div { class: "load-more", button { class: "btn ghost", onclick: move |_| { let mut f = load_more; f(()) }, "Загрузить ещё" } }
                        }
                    },
                    Some(Results::Users(p)) => rsx! {
                        div { class: "card-grid",
                            for u in p.items.iter() {
                                UserCard { key: "{u.urn}", user: u.clone() }
                            }
                        }
                        if p.next.is_some() {
                            div { class: "load-more", button { class: "btn ghost", onclick: move |_| { let mut f = load_more; f(()) }, "Загрузить ещё" } }
                        }
                    },
                    None => rsx! {},
                }
            }
        }
    }
}

// ───────────────────────────────────── жанры ─────────────────────────────────────

#[component]
pub fn GenreGrid(#[props(default = 100)] limit: usize) -> Element {
    let st = app_state();
    rsx! {
        div { class: "genre-grid",
            for (api , label , c1 , c2 , emoji) in GENRES.iter().take(limit) {
                button {
                    key: "{api}",
                    class: "genre-card",
                    "data-el": "genre_card",
                    style: "--g1: {c1}; --g2: {c2};",
                    onclick: move |_| navigate(st, Route::Genre(api.to_string())),
                    span { class: "gc-emoji", "{emoji}" }
                    span { class: "gc-label", "{label}" }
                    span { class: "gc-shine" }
                }
            }
        }
    }
}

#[component]
pub fn Genres() -> Element {
    rsx! {
        div { class: "page",
            PageHeader { title: "Жанры", kicker: "Каталог", subtitle: "Популярное на SoundCloud по жанрам и настроениям" }
            GenreGrid {}
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeedSource {
    Genre(String),
    User(String),
    Playlist(String),
    Likes,
    Stream,
}

async fn fetch_feed(src: FeedSource) -> Result<Page<Track>, String> {
    match src {
        FeedSource::Genre(g) => svc().sc.genre(&g).await,
        FeedSource::User(u) => svc().sc.user_tracks(&u).await,
        FeedSource::Playlist(p) => svc().sc.playlist_tracks(&p).await,
        FeedSource::Likes => svc().sc.likes().await,
        FeedSource::Stream => svc().sc.feed().await,
    }
}

/// Лента треков из SoundCloud с подгрузкой страниц и кнопками «Слушать».
#[component]
pub fn TrackFeed(source: FeedSource, label: String, #[props(default)] buttons: bool) -> Element {
    let mut items = use_signal(Vec::<Track>::new);
    let mut next = use_signal(|| None::<String>);
    let mut loading = use_signal(|| true);
    let mut more = use_signal(|| false);
    let mut error = use_signal(|| None::<String>);
    let src = source.clone();
    use_hook(move || {
        spawn(async move {
            match bg(fetch_feed(src)).await {
                Some(Ok(p)) => {
                    items.set(p.items);
                    next.set(p.next);
                }
                Some(Err(e)) => error.set(Some(e)),
                None => {}
            }
            loading.set(false);
        })
    });
    let load_more = move |_| {
        let Some(nx) = next.peek().clone() else { return };
        more.set(true);
        spawn(async move {
            if let Some(Ok(p)) = bg(async move { svc().sc.next_tracks(&nx).await }).await {
                items.write().extend(p.items);
                next.set(p.next);
            }
            more.set(false);
        });
    };
    if let Some(e) = error.read().clone() {
        return rsx! { ErrorBox { text: e } };
    }
    if *loading.read() {
        return rsx! { SkeletonRows {} };
    }
    let list = items.read().clone();
    rsx! {
        if buttons {
            div { class: "feed-actions", PlayButtons { tracks: list.clone(), label: label.clone() } }
        }
        TrackList { tracks: list, label: label.clone(), loading_more: *more.read(),
            on_more: if next.read().is_some() { Some(EventHandler::new(load_more)) } else { None } }
    }
}

#[component]
pub fn Genre(name: String) -> Element {
    let st = app_state();
    let g = GENRES.iter().find(|g| g.0 == name);
    let gradient = g.map(|g| (g.2.to_string(), g.3.to_string()));
    let label = genre_label(&name);
    let sc_ready = *st.sc_ready.read();
    rsx! {
        div { class: "page",
            PageHeader { title: "{label}", kicker: "Жанр", subtitle: "Свежее и популярное на SoundCloud", gradient }
            if sc_ready {
                TrackFeed { source: FeedSource::Genre(name.clone()), label: format!("Жанр: {label}"), buttons: true }
            } else {
                ErrorBox { text: "SoundCloud не подключён: укажите ключи приложения в настройках" }
            }
        }
    }
}

// ───────────────────────────────── библиотека ─────────────────────────────────

#[component]
pub fn Favorites() -> Element {
    let st = app_state();
    let items = st.library.read().favorites.clone();
    let n = items.len();
    rsx! {
        div { class: "page",
            PageHeader { title: "Избранное", kicker: "Библиотека", subtitle: "{n} треков", gradient: Some(("#ec4899".to_string(), "#8b5cf6".to_string())),
                PlayButtons { tracks: items.clone(), label: "Избранное" }
            }
            if items.is_empty() {
                Empty { icon: "heart_outline", title: "Пока пусто", text: "Нажмите ♥ у любого трека, чтобы сохранить его здесь." }
            } else {
                TrackList { tracks: items, label: "Избранное" }
            }
        }
    }
}

#[component]
pub fn History() -> Element {
    let st = app_state();
    let items = st.library.read().history.clone();
    rsx! {
        div { class: "page",
            PageHeader { title: "История", kicker: "Библиотека", subtitle: "Последние {items.len()} прослушанных треков",
                PlayButtons { tracks: items.clone(), label: "История" }
                button { class: "btn ghost", "data-el": "button",
                    onclick: move |_| {
                        let mut lib = st.library;
                        lib.write().history.clear();
                    },
                    Icon { name: "delete", size: 18 } "Очистить"
                }
            }
            if items.is_empty() {
                Empty { icon: "history", title: "История пуста", text: "Здесь появятся треки, которые вы слушали." }
            } else {
                TrackList { tracks: items, label: "История" }
            }
        }
    }
}

#[component]
pub fn Playlists() -> Element {
    let st = app_state();
    let local = st.library.read().playlists.clone();
    let logged = st.sc_user.read().is_some();
    rsx! {
        div { class: "page",
            PageHeader { title: "Плейлисты", kicker: "Библиотека",
                button { class: "btn primary", "data-el": "button",
                    onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::NewPlaylist { add: None })); },
                    Icon { name: "add", size: 18 } "Новый плейлист"
                }
            }
            Section { title: "Мои плейлисты",
                if local.is_empty() {
                    div { class: "list-empty", "Плейлистов пока нет — создайте первый." }
                } else {
                    div { class: "card-grid",
                        for p in local {
                            LocalPlaylistCard { key: "{p.id}", id: p.id.clone(), name: p.name.clone(), tracks: p.tracks.iter().take(1).cloned().collect::<Vec<_>>(), count: p.tracks.len() }
                        }
                    }
                }
            }
            if logged {
                Section { title: "Плейлисты SoundCloud", ScMyPlaylists {} }
            }
        }
    }
}

#[component]
fn LocalPlaylistCard(id: String, name: String, tracks: Vec<Track>, count: usize) -> Element {
    let st = app_state();
    let art = tracks.first().and_then(|t| t.art(ArtSize::Medium));
    rsx! {
        div { class: "track-card playlist-card", "data-el": "track_card",
            onclick: move |_| navigate(st, Route::LocalPlaylist(id.clone())),
            div { class: "tc-cover stack",
                Cover { src: art }
                div { class: "tc-play", Icon { name: "playlist", size: 22 } }
            }
            div { class: "tc-title ellipsis", "{name}" }
            div { class: "tc-artist", "{count} тр." }
        }
    }
}

#[component]
fn ScMyPlaylists() -> Element {
    let res = use_resource(|| async move { bg(async move { svc().sc.my_playlists().await }).await });
    let value = res.read().clone();
    match value {
        None => rsx! { div { class: "card-row loading", Spinner {} } },
        Some(Some(Ok(p))) => rsx! {
            div { class: "card-grid",
                for pl in p.items {
                    PlaylistCard { key: "{pl.id}", info: pl }
                }
            }
        },
        Some(Some(Err(e))) => rsx! { ErrorBox { text: e } },
        Some(None) => rsx! {},
    }
}

#[component]
pub fn LocalPlaylistPage(id: String) -> Element {
    let st = app_state();
    let pl = st.library.read().playlists.iter().find(|p| p.id == id).cloned();
    let Some(pl) = pl else {
        return rsx! { Empty { icon: "playlist", title: "Плейлист не найден" } };
    };
    let art = pl.tracks.first().and_then(|t| t.art(ArtSize::Large));
    let (id1, id2, name) = (pl.id.clone(), pl.id.clone(), pl.name.clone());
    rsx! {
        div { class: "page",
            PageHeader { title: "{pl.name}", kicker: "Плейлист", subtitle: "{pl.tracks.len()} треков", art,
                PlayButtons { tracks: pl.tracks.clone(), label: pl.name.clone() }
                IconBtn { icon: "edit", title: "Переименовать",
                    onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::RenamePlaylist { id: id1.clone(), name: name.clone() })); } }
                IconBtn { icon: "delete", title: "Удалить плейлист",
                    onclick: move |_| {
                        let mut m = st.modal;
                        m.set(Some(Modal::Confirm { title: "Удалить плейлист?".into(), text: "Треки останутся в SoundCloud и на диске.".into(), action: ConfirmAction::DeletePlaylist(id2.clone()) }));
                    } }
            }
            if pl.tracks.is_empty() {
                Empty { icon: "playlist_add", title: "Плейлист пуст", text: "Добавляйте треки через меню «…» → «В плейлист»." }
            } else {
                TrackList { tracks: pl.tracks.clone(), label: pl.name.clone(), playlist: Some(pl.id.clone()) }
            }
        }
    }
}

#[component]
pub fn ScPlaylistPage(info: PlaylistInfo) -> Element {
    let art = info.artwork.as_deref().map(|u| image_url(&sc_art_size(u, ArtSize::Large)));
    let link = info.permalink.clone();
    rsx! {
        div { class: "page",
            PageHeader { title: "{info.title}", kicker: "Плейлист SoundCloud", subtitle: "{info.user} · {info.track_count} треков", art,
                if let Some(l) = link {
                    button { class: "btn ghost", "data-el": "button", onclick: move |_| { let _ = open::that(&l); },
                        Icon { name: "external", size: 18 } "SoundCloud"
                    }
                }
            }
            TrackFeed { source: FeedSource::Playlist(info.id.clone()), label: info.title.clone(), buttons: true }
        }
    }
}

#[component]
pub fn UserPage(user: UserInfo) -> Element {
    let urn = user.urn.clone();
    let full = use_resource(move || {
        let urn = urn.clone();
        async move { bg(async move { svc().sc.user_info(&urn).await }).await }
    });
    let u = match full.read().as_ref() {
        Some(Some(Ok(u))) => u.clone(),
        _ => user.clone(),
    };
    let art = u.avatar.as_deref().map(|a| image_url(&sc_art_size(a, ArtSize::Large)));
    let sub = match (u.followers, u.track_count) {
        (Some(f), Some(t)) => format!("{} подписчиков · {} треков", fmt_count(f), t),
        (Some(f), None) => format!("{} подписчиков", fmt_count(f)),
        _ => "Исполнитель".into(),
    };
    let link = u.permalink.clone();
    rsx! {
        div { class: "page",
            PageHeader { title: "{u.username}", kicker: "Исполнитель", subtitle: sub, art, round: true,
                if let Some(l) = link {
                    button { class: "btn ghost", "data-el": "button", onclick: move |_| { let _ = open::that(&l); },
                        Icon { name: "external", size: 18 } "Открыть на SoundCloud"
                    }
                }
            }
            TrackFeed { source: FeedSource::User(user.urn.clone()), label: u.username.clone(), buttons: true }
        }
    }
}

// ───────────────────────────────── локальная музыка ─────────────────────────────────

pub fn rescan(state: AppState, notify: bool) {
    let mut scanning = state.local_scanning;
    if *scanning.peek() {
        return;
    }
    scanning.set(true);
    #[allow(unused_mut)]
    let mut folders = state.settings.peek().music_folders.clone();
    // Только для отладочной сборки: дополнительная папка для автотестов (в настройки не пишется).
    #[cfg(debug_assertions)]
    if let Ok(extra) = std::env::var("NEBULA_EXTRA_MUSIC_DIR") {
        folders.push(extra);
    }
    spawn_forever(async move {
        let idx = bg(async move { tokio::task::spawn_blocking(move || crate::library::LocalIndex::scan(&folders)).await.ok() }).await.flatten();
        if let Some(idx) = idx {
            let mut local = state.local;
            local.set(idx.tracks());
            if notify {
                toast(state, ToastKind::Success, format!("Найдено треков: {}", local.peek().len()));
            }
        }
        scanning.set(false);
    });
}

#[component]
pub fn Local() -> Element {
    let st = app_state();
    let folders = st.settings.read().music_folders.clone();
    let scanning = *st.local_scanning.read();
    let mut filter = use_signal(String::new);
    let f = filter.read().to_lowercase();
    let items: Vec<Track> = st
        .local
        .read()
        .iter()
        .filter(|t| f.is_empty() || t.title.to_lowercase().contains(&f) || t.artist.to_lowercase().contains(&f) || t.album.as_deref().unwrap_or("").to_lowercase().contains(&f))
        .cloned()
        .collect();
    let add_folder = move |_| {
        spawn(async move {
            if let Some(dir) = rfd::AsyncFileDialog::new().set_title("Папка с музыкой").pick_folder().await {
                let p = dir.path().display().to_string();
                let mut s = st.settings;
                if !s.peek().music_folders.contains(&p) {
                    s.write().music_folders.push(p);
                }
                rescan(st, true);
            }
        });
    };
    rsx! {
        div { class: "page",
            PageHeader { title: "Моя музыка", kicker: "Файлы на компьютере", subtitle: "{items.len()} треков · MP3, FLAC, WAV, OGG, M4A",
                gradient: Some(("#22d3ee".to_string(), "#6366f1".to_string())),
                PlayButtons { tracks: items.clone(), label: "Моя музыка" }
                button { class: "btn ghost", "data-el": "button", onclick: add_folder, Icon { name: "folder", size: 18 } "Добавить папку" }
                button { class: "btn ghost", "data-el": "button", disabled: scanning, onclick: move |_| rescan(st, true),
                    Icon { name: "refresh", size: 18 }
                    if scanning { "Сканирование…" } else { "Обновить" }
                }
            }
            div { class: "chips folder-chips",
                for (i , fd) in folders.iter().enumerate() {
                    span { key: "{fd}", class: "chip static", title: "{fd}",
                        Icon { name: "folder", size: 14 }
                        span { class: "ellipsis", "{fd}" }
                        button { class: "chip-x", title: "Убрать папку",
                            onclick: move |_| {
                                let mut s = st.settings;
                                s.write().music_folders.remove(i);
                                rescan(st, true);
                            },
                            "×"
                        }
                    }
                }
            }
            if folders.is_empty() {
                Empty { icon: "folder", title: "Нет папок с музыкой", text: "Добавьте папку — Nebula прочитает теги и обложки и будет следить за изменениями при каждом обновлении." }
            } else {
                div { class: "filter-bar",
                    Icon { name: "search", size: 18 }
                    input { class: "input bare", placeholder: "Фильтр по названию, исполнителю, альбому", value: "{filter}", oninput: move |e: FormEvent| filter.set(e.value()) }
                }
                if items.is_empty() && !scanning {
                    Empty { icon: "music", title: "Треков не найдено", text: "Проверьте папки или нажмите «Обновить»." }
                } else {
                    TrackList { tracks: items, label: "Моя музыка" }
                }
            }
        }
    }
}

// ───────────────────────────────── мой SoundCloud ─────────────────────────────────

#[component]
pub fn SoundCloudPage() -> Element {
    let st = app_state();
    let ready = *st.sc_ready.read();
    let user = st.sc_user.read().clone();
    let mut tab = use_signal(|| "likes");
    if !ready {
        return rsx! {
            div { class: "page",
                PageHeader { title: "Мой SoundCloud", kicker: "Аккаунт" }
                Empty { icon: "cloud", title: "SoundCloud не подключён", text: "Укажите Client ID и Client Secret своего приложения SoundCloud, затем войдите в аккаунт.",
                    button { class: "btn primary", onclick: move |_| navigate(st, Route::Settings(SettingsTab::Account)), "Открыть настройки" }
                }
            }
        };
    }
    let Some(u) = user else {
        return rsx! {
            div { class: "page",
                PageHeader { title: "Мой SoundCloud", kicker: "Аккаунт" }
                Empty { icon: "login", title: "Войдите в аккаунт", text: "После входа здесь появятся ваши лайки, лента подписок и плейлисты.",
                    button { class: "btn primary", onclick: move |_| navigate(st, Route::Settings(SettingsTab::Account)), "Войти через SoundCloud" }
                }
            }
        };
    };
    let art = u.avatar.as_deref().map(|a| image_url(&sc_art_size(a, ArtSize::Large)));
    let tabs = [("likes", "Лайки"), ("stream", "Лента"), ("playlists", "Плейлисты")];
    let current_tab: &'static str = *tab.read();
    rsx! {
        div { class: "page",
            PageHeader { title: "{u.username}", kicker: "Мой SoundCloud", art, round: true }
            div { class: "tabs",
                for (id , label) in tabs {
                    button { key: "{id}", class: if *tab.read() == id { "chip active" } else { "chip" }, "data-el": "chip", onclick: move |_| tab.set(id), "{label}" }
                }
            }
            match current_tab {
                "stream" => rsx! { TrackFeed { key: "{current_tab}", source: FeedSource::Stream, label: "Лента SoundCloud", buttons: true } },
                "playlists" => rsx! { ScMyPlaylists { key: "{current_tab}" } },
                _ => rsx! { TrackFeed { key: "{current_tab}", source: FeedSource::Likes, label: "Лайки SoundCloud", buttons: true } },
            }
        }
    }
}
