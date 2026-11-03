//! Глобальное реактивное состояние интерфейса (сигналы Dioxus), маршруты и очередь.
//! Состояние разбито на мелкие сигналы: позиция трека меняется 10 раз в секунду,
//! но перерисовывает только полосу прогресса и активную строку текста.

use crate::audio::{MediaSpec, PlaybackState};
use crate::library::LibraryData;
use crate::lyrics::Lyrics;
use crate::model::{PlaylistInfo, Track, UserInfo};
use crate::settings::Settings;
use crate::theme::Theme;
use dioxus::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    Sound,
    Playback,
    Lyrics,
    Interface,
    Themes,
    Layout,
    Elements,
    Account,
    Cache,
    Hotkeys,
    About,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 11] = [
        SettingsTab::Sound,
        SettingsTab::Playback,
        SettingsTab::Lyrics,
        SettingsTab::Interface,
        SettingsTab::Themes,
        SettingsTab::Layout,
        SettingsTab::Elements,
        SettingsTab::Account,
        SettingsTab::Cache,
        SettingsTab::Hotkeys,
        SettingsTab::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SettingsTab::Sound => "Звук",
            SettingsTab::Playback => "Воспроизведение",
            SettingsTab::Lyrics => "Текст песен",
            SettingsTab::Interface => "Интерфейс",
            SettingsTab::Themes => "Темы",
            SettingsTab::Layout => "Расположение",
            SettingsTab::Elements => "Элементы",
            SettingsTab::Account => "SoundCloud",
            SettingsTab::Cache => "Кэш и данные",
            SettingsTab::Hotkeys => "Горячие клавиши",
            SettingsTab::About => "О программе",
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            SettingsTab::Sound => "tune",
            SettingsTab::Playback => "play",
            SettingsTab::Lyrics => "lyrics",
            SettingsTab::Interface => "sparkles",
            SettingsTab::Themes => "palette",
            SettingsTab::Layout => "layout",
            SettingsTab::Elements => "brush",
            SettingsTab::Account => "cloud",
            SettingsTab::Cache => "storage",
            SettingsTab::Hotkeys => "keyboard",
            SettingsTab::About => "info",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Home,
    Search,
    Genres,
    Genre(String),
    Favorites,
    History,
    Playlists,
    LocalPlaylist(String),
    ScPlaylist(PlaylistInfo),
    User(UserInfo),
    Local,
    SoundCloud,
    Settings(SettingsTab),
}

impl Route {
    pub fn key(&self) -> String {
        match self {
            Route::Home => "home".into(),
            Route::Search => "search".into(),
            Route::Genres => "genres".into(),
            Route::Genre(g) => format!("genre:{g}"),
            Route::Favorites => "favorites".into(),
            Route::History => "history".into(),
            Route::Playlists => "playlists".into(),
            Route::LocalPlaylist(id) => format!("lp:{id}"),
            Route::ScPlaylist(p) => format!("sp:{}", p.id),
            Route::User(u) => format!("user:{}", u.urn),
            Route::Local => "local".into(),
            Route::SoundCloud => "soundcloud".into(),
            Route::Settings(t) => format!("settings:{t:?}"),
        }
    }

    /// Пункт бокового меню, который подсвечивается для этого маршрута.
    pub fn nav_id(&self) -> &'static str {
        match self {
            Route::Home => "home",
            Route::Search => "search",
            Route::Genres | Route::Genre(_) => "genres",
            Route::Favorites => "favorites",
            Route::History => "history",
            Route::Playlists | Route::LocalPlaylist(_) | Route::ScPlaylist(_) => "playlists",
            Route::User(_) => "search",
            Route::Local => "local",
            Route::SoundCloud => "soundcloud",
            Route::Settings(_) => "settings",
        }
    }

    pub fn from_nav(id: &str) -> Route {
        match id {
            "search" => Route::Search,
            "genres" => Route::Genres,
            "favorites" => Route::Favorites,
            "history" => Route::History,
            "playlists" => Route::Playlists,
            "local" => Route::Local,
            "soundcloud" => Route::SoundCloud,
            "settings" => Route::Settings(SettingsTab::Sound),
            _ => Route::Home,
        }
    }
}

/// Очередь воспроизведения. Перемешивание физически переставляет треки после
/// текущего; исходный порядок хранится, чтобы вернуть его при выключении.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Queue {
    pub tracks: Vec<Track>,
    pub index: Option<usize>,
    pub original: Option<Vec<Track>>,
    pub label: String,
}

pub fn shuffle_in_place<T>(v: &mut [T]) {
    let mut seed = {
        let mut b = [0u8; 8];
        let _ = getrandom::fill(&mut b);
        u64::from_le_bytes(b) | 1
    };
    for i in (1..v.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        let j = (seed % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
}

impl Queue {
    pub fn current(&self) -> Option<&Track> {
        self.index.and_then(|i| self.tracks.get(i))
    }

    pub fn set(&mut self, tracks: Vec<Track>, start: usize, shuffle: bool, label: &str) {
        self.tracks = tracks;
        self.index = if self.tracks.is_empty() { None } else { Some(start.min(self.tracks.len() - 1)) };
        self.original = None;
        self.label = label.to_string();
        if shuffle {
            self.shuffle_on();
        }
    }

    pub fn shuffle_on(&mut self) {
        if self.original.is_some() || self.tracks.len() < 2 {
            return;
        }
        self.original = Some(self.tracks.clone());
        let cur = self.index.unwrap_or(0);
        let current = self.tracks.remove(cur);
        shuffle_in_place(&mut self.tracks);
        self.tracks.insert(0, current);
        self.index = Some(0);
    }

    pub fn shuffle_off(&mut self) {
        if let Some(orig) = self.original.take() {
            let cur_id = self.current().map(|t| t.id.clone());
            // треки, добавленные во время перемешивания, сохраняем в конце
            let mut restored = orig;
            for t in &self.tracks {
                if !restored.iter().any(|x| x.id == t.id) {
                    restored.push(t.clone());
                }
            }
            restored.retain(|t| self.tracks.iter().any(|x| x.id == t.id));
            self.tracks = restored;
            self.index = cur_id.and_then(|id| self.tracks.iter().position(|t| t.id == id)).or(self.index);
        }
    }

    pub fn insert_next(&mut self, t: Track) {
        let at = self.index.map(|i| i + 1).unwrap_or(0).min(self.tracks.len());
        self.tracks.insert(at, t);
        if self.index.is_none() {
            self.index = Some(0);
        }
    }

    pub fn append(&mut self, t: Track) {
        self.tracks.push(t);
        if self.index.is_none() {
            self.index = Some(0);
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i >= self.tracks.len() {
            return;
        }
        let removed = self.tracks.remove(i);
        if let Some(orig) = &mut self.original {
            orig.retain(|t| t.id != removed.id);
        }
        self.index = match self.index {
            None => None,
            _ if self.tracks.is_empty() => None,
            Some(cur) if i < cur => Some(cur - 1),
            Some(cur) => Some(cur.min(self.tracks.len() - 1)),
        };
    }

    pub fn move_item(&mut self, from: usize, to: usize) {
        if from >= self.tracks.len() || to >= self.tracks.len() || from == to {
            return;
        }
        let t = self.tracks.remove(from);
        self.tracks.insert(to, t);
        if let Some(cur) = self.index {
            self.index = Some(if cur == from {
                to
            } else if from < cur && to >= cur {
                cur - 1
            } else if from > cur && to <= cur {
                cur + 1
            } else {
                cur
            });
        }
    }

    pub fn clear_upcoming(&mut self) {
        if let Some(i) = self.index {
            self.tracks.truncate(i + 1);
        } else {
            self.tracks.clear();
        }
        self.original = None;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LyricsState {
    Idle,
    Loading,
    NotFound,
    Found(Lyrics),
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MenuState {
    pub x: f64,
    pub y: f64,
    pub track: Track,
    /// Если меню открыто в локальном плейлисте — можно удалить трек из него.
    pub playlist: Option<String>,
    /// Индекс в очереди, если меню открыто из панели очереди.
    pub queue_index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Modal {
    NewPlaylist { add: Option<Track> },
    RenamePlaylist { id: String, name: String },
    AddToPlaylist { track: Track },
    LyricsSearch,
    Confirm { title: String, text: String, action: ConfirmAction },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfirmAction {
    DeletePlaylist(String),
    ClearCache,
    ResetSettings,
    DeleteTheme(String),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Preloaded {
    pub track_id: String,
    pub spec: Option<MediaSpec>,
    pub preview: bool,
    pub at: Option<std::time::Instant>,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub settings: Signal<Settings>,
    pub theme: Signal<Theme>,
    pub themes: Signal<Vec<Theme>>,
    pub library: Signal<LibraryData>,
    pub local: Signal<Vec<Track>>,
    pub local_scanning: Signal<bool>,
    pub route: Signal<Route>,
    pub back: Signal<Vec<Route>>,
    pub forward: Signal<Vec<Route>>,
    pub queue: Signal<Queue>,
    pub now: Signal<Option<Track>>,
    pub load_id: Signal<u64>,
    pub pstate: Signal<PlaybackState>,
    pub position: Signal<u64>,
    pub duration: Signal<u64>,
    pub buffered: Signal<f32>,
    pub device: Signal<String>,
    pub preview: Signal<bool>,
    pub preloaded: Signal<Preloaded>,
    pub restore_pos: Signal<Option<u64>>,
    pub lyrics: Signal<LyricsState>,
    pub lyrics_offset: Signal<i32>,
    pub fullscreen: Signal<bool>,
    pub eq_open: Signal<bool>,
    pub edit_mode: Signal<bool>,
    pub selected_el: Signal<Option<String>>,
    pub toasts: Signal<Vec<Toast>>,
    pub menu: Signal<Option<MenuState>>,
    pub modal: Signal<Option<Modal>>,
    pub accent: Signal<Option<String>>,
    pub focused: Signal<bool>,
    pub hidden: Signal<bool>,
    pub level: Signal<f32>,
    pub sc_ready: Signal<bool>,
    pub sc_user: Signal<Option<UserInfo>>,
    /// Открыто окно входа в SoundCloud и мы ждём сессию.
    pub sc_login_busy: Signal<bool>,
    pub closing: Signal<bool>,
    pub reopening: Signal<bool>,
    pub search_query: Signal<String>,
    pub errors_in_row: Signal<u32>,
    pub related: Signal<Option<(String, Vec<Track>)>>,
}

pub fn app_state() -> AppState {
    use_context::<AppState>()
}
