//! Страницы: главная, поиск, жанры, избранное, история, плейлисты,
//! исполнитель, локальная музыка и личный SoundCloud.

use super::actions::*;
use super::icons::Icon;
use super::player_bar::Cover;
use super::sc_account::LoginButton;
use super::state::*;
use super::tracks::*;
use super::widgets::{use_switch_dir, Empty, IconBtn, Spinner};
use crate::model::{fmt_count, fmt_listening, image_url, sc_art_size, ArtSize, Page, PlaylistInfo, Track, UserInfo};
use crate::services::svc;
use crate::theme::HOME_SECTIONS;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::time::Duration;

/// (жанр в API, подпись, значок). Цвета у жанров нет: карточка красится
/// оттенком из палитры темы (см. [`genre_tint`]), поэтому каталог подходит к любой теме.
pub const GENRES: &[(&str, &str, &str)] = &[
    ("Phonk", "Фонк", "g_car"),
    ("Hip-hop & Rap", "Хип-хоп и рэп", "mic"),
    ("Electronic", "Электроника", "g_bolt"),
    ("House", "Хаус", "g_speaker"),
    ("Techno", "Техно", "g_chip"),
    ("Drum & Bass", "Drum & Bass", "g_speed"),
    ("Lo-fi", "Lo-Fi", "g_cafe"),
    ("Ambient", "Эмбиент", "g_landscape"),
    ("Synthwave", "Синтвейв", "g_city"),
    ("Trap", "Трэп", "g_diamond"),
    ("Pop", "Поп", "g_star"),
    ("Rock", "Рок", "g_album"),
    ("Metal", "Метал", "g_flame"),
    ("R&B & Soul", "R&B и соул", "heart"),
    ("Jazz & Blues", "Джаз и блюз", "g_nightlife"),
    ("Classical", "Классика", "g_piano"),
    ("Dubstep", "Дабстеп", "g_eq"),
    ("Deep House", "Дип-хаус", "g_headset"),
    ("Dance & EDM", "Dance и EDM", "sparkles"),
    ("Trance", "Транс", "g_infinity"),
    ("Indie", "Инди", "g_flower"),
    ("Chill", "Чилл", "g_moon"),
    ("Soundtrack", "Саундтреки", "g_movie"),
    ("Reggaeton", "Реггетон", "g_sun"),
];

fn genre_label(api: &str) -> String {
    GENRES.iter().find(|g| g.0 == api).map(|g| g.1.to_string()).unwrap_or_else(|| api.to_string())
}

/// Оттенок жанра — доля второго акцента темы в смеси с первым (0–100 %).
/// Шаг «золотого сечения» раскладывает оттенки равномерно при любом числе
/// карточек: соседние различаются, а у жанра везде один и тот же цвет.
fn genre_tint(api: &str) -> u32 {
    let i = GENRES.iter().position(|g| g.0 == api).unwrap_or(0);
    ((i as f64 * 0.618_034).fract() * 100.0).round() as u32
}

/// CSS-цвет жанра из палитры текущей темы.
fn genre_color(api: &str) -> String {
    format!("color-mix(in oklab, var(--accent2) {}%, var(--accent))", genre_tint(api))
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
    // нет сессии или она истекла — сразу предлагаем войти
    let needs_login = text.contains("не подключён") || text.contains("войдите");
    rsx! {
        div { class: "error-box glass",
            Icon { name: "warning", size: 20 }
            span { "{text}" }
            if needs_login {
                LoginButton { class: "small" }
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
                    "listening" => rsx! { ListeningSection { key: "{entry.id}" } },
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
            div { class: "home-foot", "Nebula · SoundCloud, Яндекс Музыка и LRCLIB" }
            // упоминание HOME_SECTIONS для редактора раскладки
            span { class: "hidden", "{HOME_SECTIONS.len()}" }
        }
    }
}

/// Сколько визуализаторов сейчас на экране (уходящая страница может
/// исчезать, пока новая уже показала свой) — анализатор работает, пока > 0.
static VISUALIZERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Принимает полосы спектра и выставляет высоту столбиков текущей страницы.
const CAVA_JS: &str = r#"
while (true) {
  const v = await dioxus.recv();
  const bars = document.querySelectorAll(".page-wrap.current .cava i");
  for (let i = 0; i < bars.length; i++) bars[i].style.setProperty("--h", (v[i] || 0).toFixed(3));
}
"#;

/// Визуализатор спектра в духе cava (github.com/karlstav/cava) внутри
/// планеты на главной: 24 столбика от низких частот к высоким, растут от
/// центра. Данные — анализатор в аудиоколбэке, ~30 обновлений в секунду;
/// в тишине и на паузе ничего не отправляется.
#[component]
fn Visualizer() -> Element {
    use std::sync::atomic::Ordering;
    use_hook(|| {
        VISUALIZERS.fetch_add(1, Ordering::Relaxed);
        svc().player.set_visualizer(true);
    });
    use_drop(|| {
        if VISUALIZERS.fetch_sub(1, Ordering::Relaxed) <= 1 {
            svc().player.set_visualizer(false);
        }
    });
    use_future(|| async move {
        let ev = document::eval(CAVA_JS);
        let mut quiet = 0u32;
        loop {
            tokio::time::sleep(Duration::from_millis(33)).await;
            let v = svc().player.spectrum();
            if v.iter().all(|x| *x < 0.004) {
                quiet += 1;
                // пару кадров нулей — чтобы столбики опустились, дальше тишина
                if quiet > 3 {
                    continue;
                }
            } else {
                quiet = 0;
            }
            let vals: Vec<f32> = v.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
            let _ = ev.send(vals);
        }
    });
    rsx! {
        div { class: "cava", "aria-hidden": "true",
            for i in 0..crate::audio::SPECTRUM_BANDS {
                i { key: "{i}" }
            }
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
    let logged_in = st.sc_user.read().is_some();
    // в приветствии — никнейм аккаунта SoundCloud, без входа — «путешественник»
    let name = st.sc_user.read().as_ref().map(|u| u.username.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| "путешественник".to_string());
    let viz = {
        let s = st.settings.read();
        s.visualizer && !s.low_power
    };
    // статистика в шапке подчиняется переключателю «Время прослушивания»
    let stats_on = st.theme.read().layout.home.iter().find(|e| e.id == "listening").is_none_or(|e| e.visible);
    rsx! {
        section { class: "hero glass", "data-el": "hero",
            div { class: "hero-orbit", span {} span {} span {}
                if viz {
                    Visualizer {}
                }
            }
            div { class: "hero-text",
                h1 { class: "hero-title", "{greeting}, {name}" }
                p { class: "hero-sub", "Музыка из SoundCloud, YouTube и с вашего компьютера, синхронные тексты из LRCLIB и интерфейс, который меняется как вам угодно." }
                if stats_on {
                    ListeningStats { class: "in-hero" }
                }
                if !logged_in {
                    div { class: "hero-note",
                        Icon { name: "cloud", size: 18 }
                        span { "Слушать и искать можно без входа. Войдите в SoundCloud, если нужны ваши лайки, лента и плейлисты." }
                        LoginButton { class: "small" }
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

/// Отдельный раздел «Время прослушивания» — только если шапка главной скрыта:
/// иначе та же статистика уже показана в шапке.
#[component]
fn ListeningSection() -> Element {
    let st = app_state();
    let hero_visible = st.theme.read().layout.home.iter().any(|e| e.id == "hero" && e.visible);
    if hero_visible {
        return rsx! {};
    }
    rsx! {
        Section { title: "Время прослушивания", ListeningStats {} }
    }
}

/// Статистика прослушивания: сколько всего наслушано в Nebula и любимый трек.
#[component]
fn ListeningStats(#[props(default)] class: String) -> Element {
    let st = app_state();
    let library = st.library.read().clone();
    // время привязано к аккаунту SoundCloud; без входа — гостевая статистика
    let account = st.sc_user.read().as_ref().filter(|u| !u.urn.is_empty()).map(|u| (u.urn.clone(), u.username.trim().to_string()));
    let stats = library.listening_for(account.as_ref().map(|(urn, _)| urn.as_str()));
    let label = match &account {
        Some((_, name)) if !name.is_empty() => format!("Всего · {name}"),
        _ => "Всего в Nebula".to_string(),
    };
    let total = stats.total_with_imported();
    let top = stats
        .by_track_ms
        .iter()
        .max_by_key(|(_, ms)| **ms)
        .and_then(|(id, ms)| {
            library
                .history
                .iter()
                .chain(library.favorites.iter())
                .find(|t| &t.id == id)
                .cloned()
                .map(|t| (t, *ms))
        });
    rsx! {
        div { class: "listening-panel glass {class}", "data-el": "listening_stats",
                div { class: "listening-total",
                    div { class: "listening-label", "{label}" }
                    strong { "{fmt_listening(total)}" }
                    span { class: "listening-caption",
                        if total == 0 { "Начните слушать — статистика обновится автоматически" } else { "Счётчик учитывает только реально проигранное время" }
                    }
                }
                div { class: "listening-divider" }
                div { class: "listening-detail",
                    div { class: "listening-label", "Треков в статистике" }
                    strong { "{stats.by_track_ms.len()}" }
                    if let Some((track, ms)) = top {
                        div { class: "listening-top ellipsis", title: "{track.title}", "Чаще всего: {track.title} · {fmt_listening(ms)}" }
                    } else {
                        div { class: "listening-top", "Самый любимый трек появится здесь" }
                    }
                }
                if stats.imported_ms > 0 {
                    span { class: "listening-imported", "Импортировано: {fmt_listening(stats.imported_ms)}" }
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
                    "youtube" => svc().yt.search(&q2).await.map(Results::Tracks),
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
                    "youtube" => svc().yt.next_tracks(&n2).await.map(Results::Tracks),
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
    let tabs = [("tracks", "Треки"), ("playlists", "Плейлисты"), ("users", "Исполнители"), ("youtube", "YouTube"), ("local", "Моя музыка")];

    // Содержимое под вкладками въезжает слева/справа при смене вкладки.
    // Ключ — вкладка, а при пустом запросе — «idle»: подсказки не дёргаются,
    // когда вкладку переключают, ещё ничего не набрав.
    let tab_now: &'static str = *tab.read();
    let tab_index = tabs.iter().position(|t| t.0 == tab_now).unwrap_or(0);
    let dir = use_switch_dir(tab_index, "tp-l", "tp-r", "tp-first");
    let idle = q_now.trim().is_empty();
    let pane_key = if idle { "idle" } else { tab_now };
    let pane_cls = if idle { "tp-idle" } else { dir };
    let pane = rsx! {
        div { key: "{pane_key}", class: "tab-pane {pane_cls}",
            if q_now.trim().is_empty() {
                if !recent.is_empty() {
                    section { class: "section",
                        div { class: "section-head",
                            h2 { class: "section-title", "data-el": "section_title", "Вы искали" }
                            button { class: "link-btn", title: "Удалить всю историю поиска",
                                onclick: move |_| {
                                    let mut lib = st.library;
                                    lib.write().recent_searches.clear();
                                    toast(st, ToastKind::Success, "История поиска очищена");
                                },
                                "Очистить"
                            }
                        }
                        div { class: "chips",
                            for r in recent {
                                span { key: "{r}", class: "chip removable", "data-el": "chip",
                                    button { class: "chip-main", onclick: { let r = r.clone(); move |_| query.set(r.clone()) }, "{r}" }
                                    button { class: "chip-x", title: "Убрать из истории",
                                        onclick: {
                                            let r = r.clone();
                                            move |_| {
                                                let mut lib = st.library;
                                                lib.write().recent_searches.retain(|x| x != &r);
                                            }
                                        },
                                        "×"
                                    }
                                }
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
                            TrackList { tracks: p.items.clone(), label: if tab_now == "youtube" { format!("YouTube: {q_now}") } else { format!("Поиск: {q_now}") }, loading_more: *more_loading.read(),
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
    };

    rsx! {
        div { class: "page search",
            div { class: "search-bar glass", "data-el": "search",
                Icon { name: "search", size: 22 }
                input {
                    id: "search-input",
                    class: "search-input",
                    autofocus: true,
                    // без всплывающих «Сохранённых сведений» браузера — своя история есть ниже
                    autocomplete: "off",
                    spellcheck: false,
                    placeholder: "Что будем слушать?",
                    value: "{q_now}",
                    oninput: move |e: FormEvent| query.set(e.value()),
                }
                if !q_now.is_empty() {
                    IconBtn { icon: "close", size: 18, title: "Очистить", onclick: move |_| query.set(String::new()) }
                }
            }
            div { class: "tabs", "data-glide": ".chip.active",
                span { class: "glider chip-glider" }
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
            {pane}
        }
    }
}

// ───────────────────────────────────── жанры ─────────────────────────────────────

#[component]
pub fn GenreGrid(#[props(default = 100)] limit: usize) -> Element {
    let st = app_state();
    let cards: Vec<(&'static str, &'static str, &'static str, u32)> = GENRES.iter().take(limit).map(|g| (g.0, g.1, g.2, genre_tint(g.0))).collect();
    rsx! {
        div { class: "genre-grid",
            for (api , label , icon , tint) in cards {
                button {
                    key: "{api}",
                    class: "genre-card",
                    "data-el": "genre_card",
                    style: "--t: {tint}%;",
                    onclick: move |_| navigate(st, Route::Genre(api.to_string())),
                    span { class: "gc-mark", Icon { name: icon.to_string(), size: 88 } }
                    span { class: "gc-icon", Icon { name: icon.to_string(), size: 20 } }
                    span { class: "gc-label", "{label}" }
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
    // заголовок подкрашен тем же оттенком темы, что и карточка жанра
    let gradient = g.map(|g| (genre_color(g.0), "var(--accent2)".to_string()));
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
            PageHeader { title: "Избранное", kicker: "Библиотека", subtitle: "{n} треков",
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
                button { class: "btn ghost", "data-el": "button", title: "Перенести плейлист из Яндекс Музыки или Spotify",
                    onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::Import)); },
                    Icon { name: "download", size: 18 } "Импортировать"
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
    let tabs = [("likes", "Лайки"), ("stream", "Лента"), ("playlists", "Плейлисты")];
    let current_tab: &'static str = *tab.read();
    let dir = use_switch_dir(tabs.iter().position(|t| t.0 == current_tab).unwrap_or(0), "tp-l", "tp-r", "tp-first");
    let Some(u) = user.filter(|_| ready) else {
        return rsx! {
            div { class: "page",
                PageHeader { title: "Мой SoundCloud", kicker: "Аккаунт" }
                Empty { icon: "cloud", title: "Войдите в SoundCloud", text: "Откроется окно со страницей входа SoundCloud — как на сайте. После входа здесь появятся ваши лайки, лента подписок и плейлисты.",
                    LoginButton { class: "big" }
                }
            }
        };
    };
    let art = u.avatar.as_deref().map(|a| image_url(&sc_art_size(a, ArtSize::Large)));
    // ключ на корне rsx-блока: смена вкладки пересоздаёт и анимирует содержимое
    let pane = rsx! {
        div { key: "{current_tab}", class: "tab-pane {dir}",
            match current_tab {
                "stream" => rsx! { TrackFeed { source: FeedSource::Stream, label: "Лента SoundCloud", buttons: true } },
                "playlists" => rsx! { ScMyPlaylists {} },
                _ => rsx! { TrackFeed { source: FeedSource::Likes, label: "Лайки SoundCloud", buttons: true } },
            }
        }
    };
    rsx! {
        div { class: "page",
            PageHeader { title: "{u.username}", kicker: "Мой SoundCloud", art, round: true }
            div { class: "tabs", "data-glide": ".chip.active",
                span { class: "glider chip-glider" }
                for (id , label) in tabs {
                    button { key: "{id}", class: if *tab.read() == id { "chip active" } else { "chip" }, "data-el": "chip", onclick: move |_| tab.set(id), "{label}" }
                }
            }
            {pane}
        }
    }
}
