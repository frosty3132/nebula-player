//! Списки и карточки: строки треков, сетки карточек, плейлисты, исполнители.
//! Картинки грузятся лениво (`loading=lazy`) и кэшируются на диске обработчиком `/media`.

use super::actions::*;
use super::icons::Icon;
use super::player_bar::Cover;
use super::state::*;
use super::widgets::IconBtn;
use crate::audio::PlaybackState;
use crate::model::{fmt_count, fmt_time, image_url, sc_art_size, ArtSize, PlaylistInfo, Track, UserInfo};
use dioxus::prelude::*;

#[component]
pub fn PlayingBars() -> Element {
    rsx! {
        span { class: "bars", span {} span {} span {} }
    }
}

#[component]
pub fn TrackList(
    tracks: Vec<Track>,
    #[props(default)] label: String,
    #[props(default)] playlist: Option<String>,
    #[props(default)] loading_more: bool,
    #[props(default)] on_more: Option<EventHandler<()>>,
) -> Element {
    let st = app_state();
    let numbers = st.settings.read().show_track_numbers;
    let n = tracks.len();
    // один колбэк на весь список вместо копии списка в каждой строке
    let (all, lbl) = (tracks.clone(), label.clone());
    let play_at = use_callback(move |i: usize| play_list(st, all.clone(), i, &lbl));
    // превью в списке заранее подбираются на YouTube — по клику звук начнётся быстрее
    let replace = st.settings.read().yt_replace_previews;
    let shown = tracks.clone();
    use_effect(use_reactive!(|shown, replace| {
        if replace {
            crate::services::svc().yt.prematch(shown);
        }
    }));
    rsx! {
        div { class: "track-list",
            for (i , t) in tracks.iter().enumerate() {
                TrackRow {
                    key: "{t.id}-{i}",
                    track: t.clone(),
                    index: i,
                    numbers,
                    playlist: playlist.clone(),
                    on_play: move |_| play_at.call(i),
                }
            }
            if let Some(more) = on_more {
                div { class: "load-more",
                    button {
                        class: "btn ghost",
                        "data-el": "button",
                        disabled: loading_more,
                        onclick: move |_| more.call(()),
                        if loading_more { "Загрузка…" } else { "Загрузить ещё" }
                    }
                }
            }
            if n == 0 {
                div { class: "list-empty", "Здесь пока пусто" }
            }
        }
    }
}

#[component]
pub fn TrackRow(track: Track, index: usize, numbers: bool, playlist: Option<String>, on_play: EventHandler<()>) -> Element {
    let st = app_state();
    let is_now = st.now.read().as_ref().is_some_and(|t| t.id == track.id);
    let pstate = *st.pstate.read();
    let playing = is_now && matches!(pstate, PlaybackState::Playing | PlaybackState::Buffering);
    let liked = st.library.read().is_favorite(&track.id);
    let art = track.art(ArtSize::Small);
    let uploader = track.uploader_urn.clone().map(|urn| UserInfo { urn, username: track.uploader.clone(), ..Default::default() });
    let (t1, t2, t3) = (track.clone(), track.clone(), track.clone());
    let pl1 = playlist.clone();
    let pl2 = playlist.clone();
    let yt_replace = st.settings.read().yt_replace_previews;
    // недоступный на SoundCloud трек сыграет с YouTube, если включена замена
    let unavailable = track.is_blocked() && !yt_replace;
    let class = format!(
        "track-row{}{}{}",
        if is_now { " now" } else { "" },
        if playing { " playing" } else { "" },
        if unavailable { " blocked" } else { "" }
    );
    rsx! {
        div {
            class: "{class}",
            "data-el": "track_row",
            ondoubleclick: move |_| on_play.call(()),
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let p = e.client_coordinates();
                let mut m = st.menu;
                m.set(Some(MenuState { x: p.x, y: p.y, track: t1.clone(), playlist: pl1.clone(), queue_index: None }));
            },
            div { class: "tr-index",
                if playing {
                    PlayingBars {}
                } else if numbers {
                    span { class: "tr-num", "{index + 1}" }
                }
                button {
                    class: "tr-play",
                    title: if is_now && playing { "Пауза" } else { "Играть" },
                    onclick: move |e| {
                        e.stop_propagation();
                        if is_now { toggle_play(st) } else { on_play.call(()) }
                    },
                    Icon { name: if is_now && playing { "pause" } else { "play" }, size: 18 }
                }
            }
            Cover { src: art, class: "tr-cover" }
            div { class: "tr-main",
                div { class: "tr-title",
                    span { class: "ellipsis", "{track.title}" }
                    if track.id.starts_with("yt:") {
                        span { class: "badge soft", title: "Трек с YouTube", "YouTube" }
                    } else if (track.is_preview() || track.is_blocked()) && yt_replace {
                        span { class: "badge soft", title: "На SoundCloud только превью — сыграет полная версия с YouTube", "полный · YT" }
                    } else if track.is_preview() {
                        span { class: "badge", "превью" }
                    }
                    if track.is_local() {
                        span { class: "badge soft", "файл" }
                    }
                }
                div {
                    class: "tr-artist ellipsis",
                    onclick: move |e| {
                        if let Some(u) = uploader.clone() {
                            e.stop_propagation();
                            navigate(st, Route::User(u));
                        }
                    },
                    "{track.artist}"
                }
            }
            div { class: "tr-genre ellipsis",
                if let Some(a) = &track.album {
                    "{a}"
                } else if let Some(g) = &track.genre {
                    "{g}"
                }
            }
            div { class: "tr-stats",
                if let Some(p) = track.plays {
                    span { title: "Прослушивания", "▶ {fmt_count(p)}" }
                }
            }
            button {
                class: if liked { "icon-btn like on" } else { "icon-btn like" },
                "data-el": "button",
                onclick: move |e| {
                    e.stop_propagation();
                    toggle_like(st, &t2)
                },
                Icon { name: if liked { "heart" } else { "heart_outline" }, size: 17 }
            }
            div { class: "tr-dur", "{fmt_time(track.duration_ms)}" }
            IconBtn {
                icon: "more",
                size: 18,
                title: "Ещё",
                onclick: move |e: MouseEvent| {
                    let p = e.client_coordinates();
                    let mut m = st.menu;
                    m.set(Some(MenuState { x: p.x, y: p.y, track: t3.clone(), playlist: pl2.clone(), queue_index: None }));
                },
            }
        }
    }
}

#[component]
pub fn TrackCard(track: Track, on_play: EventHandler<()>) -> Element {
    let st = app_state();
    let is_now = st.now.read().as_ref().is_some_and(|t| t.id == track.id);
    let playing = is_now && matches!(*st.pstate.read(), PlaybackState::Playing | PlaybackState::Buffering);
    let art = track.art(ArtSize::Medium);
    let t1 = track.clone();
    rsx! {
        div {
            class: if is_now { "track-card now" } else { "track-card" },
            "data-el": "track_card",
            onclick: move |_| {
                if is_now { toggle_play(st) } else { on_play.call(()) }
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let p = e.client_coordinates();
                let mut m = st.menu;
                m.set(Some(MenuState { x: p.x, y: p.y, track: t1.clone(), playlist: None, queue_index: None }));
            },
            div { class: "tc-cover",
                Cover { src: art }
                div { class: "tc-play",
                    Icon { name: if playing { "pause" } else { "play" }, size: 22 }
                }
                if playing {
                    div { class: "tc-eq", PlayingBars {} }
                }
            }
            div { class: "tc-title ellipsis", title: "{track.title}", "{track.title}" }
            div { class: "tc-artist ellipsis", "{track.artist}" }
        }
    }
}

/// Горизонтальная лента карточек с прокруткой колесом.
#[component]
pub fn CardRow(tracks: Vec<Track>, #[props(default)] label: String, #[props(default)] grid: bool) -> Element {
    let st = app_state();
    let (all, lbl) = (tracks.clone(), label.clone());
    let play_at = use_callback(move |i: usize| play_list(st, all.clone(), i, &lbl));
    rsx! {
        div { class: if grid { "card-grid" } else { "card-row" },
            for (i , t) in tracks.iter().enumerate() {
                TrackCard { key: "{t.id}-{i}", track: t.clone(), on_play: move |_| play_at.call(i) }
            }
        }
    }
}

#[component]
pub fn PlaylistCard(info: PlaylistInfo) -> Element {
    let st = app_state();
    let art = info.artwork.as_deref().map(|u| image_url(&sc_art_size(u, ArtSize::Medium)));
    let i2 = info.clone();
    rsx! {
        div { class: "track-card playlist-card", "data-el": "track_card",
            onclick: move |_| navigate(st, Route::ScPlaylist(i2.clone())),
            div { class: "tc-cover stack",
                Cover { src: art }
                div { class: "tc-play", Icon { name: "playlist", size: 22 } }
            }
            div { class: "tc-title ellipsis", "{info.title}" }
            div { class: "tc-artist ellipsis", "{info.user} · {info.track_count} тр." }
        }
    }
}

#[component]
pub fn UserCard(user: UserInfo) -> Element {
    let st = app_state();
    let art = user.avatar.as_deref().map(|u| image_url(&sc_art_size(u, ArtSize::Medium)));
    let u2 = user.clone();
    rsx! {
        div { class: "track-card user-card", "data-el": "track_card",
            onclick: move |_| navigate(st, Route::User(u2.clone())),
            div { class: "tc-cover round", Cover { src: art } }
            div { class: "tc-title ellipsis", "{user.username}" }
            div { class: "tc-artist ellipsis",
                if let Some(f) = user.followers { "{fmt_count(f)} подписчиков" } else { "исполнитель" }
            }
        }
    }
}

#[component]
pub fn SkeletonRows(#[props(default = 8)] count: usize) -> Element {
    rsx! {
        div { class: "track-list",
            for i in 0..count {
                div { key: "{i}", class: "track-row skeleton",
                    div { class: "sk sk-cover" }
                    div { class: "tr-main", div { class: "sk sk-line" } div { class: "sk sk-line short" } }
                }
            }
        }
    }
}
