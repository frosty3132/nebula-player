//! Темы: цвета, форма, стекло, шрифт, фон, звёзды, раскладка интерфейса и
//! индивидуальные стили КАЖДОГО элемента (через `data-el`), плюс свой CSS.
//! Тема целиком компилируется в CSS-переменные и правила, которые
//! подставляются в `<style>` — никакого JS, только браузерный CSS-движок.

use crate::paths::{paths, read_json, write_json};
use crate::settings::Settings;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub bg: String,
    pub surface: String,
    pub surface2: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
    pub accent2: String,
    pub glow: String,
    pub danger: String,
    pub border: String,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            bg: "#05030f".into(),
            surface: "#140f2b".into(),
            surface2: "#1f1745".into(),
            text: "#ece8ff".into(),
            muted: "#9b94c9".into(),
            accent: "#8b5cf6".into(),
            accent2: "#22d3ee".into(),
            glow: "#a78bfa".into(),
            danger: "#fb7185".into(),
            border: "rgba(255,255,255,0.09)".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shape {
    pub radius: f32,
    pub radius_small: f32,
    pub radius_large: f32,
    pub border_width: f32,
    pub cover_radius: f32,
    pub button_radius: f32,
}

impl Default for Shape {
    fn default() -> Self {
        Self { radius: 18.0, radius_small: 11.0, radius_large: 26.0, border_width: 1.0, cover_radius: 12.0, button_radius: 999.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Glass {
    pub blur: f32,
    pub opacity: f32,
    pub saturation: f32,
    /// Непрозрачность фона окна: < 1 — сквозь окно виден рабочий стол.
    pub window_opacity: f32,
    pub glow: f32,
    /// «Жидкое стекло» как в iOS: молочная подложка, световой край, глубокая тень.
    pub liquid: bool,
}

impl Default for Glass {
    fn default() -> Self {
        Self { blur: 22.0, opacity: 0.55, saturation: 150.0, window_opacity: 0.93, glow: 0.55, liquid: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Font {
    pub family: String,
    pub size: f32,
    pub title_weight: u16,
    pub letter_spacing: f32,
}

impl Default for Font {
    fn default() -> Self {
        Self { family: "\"Segoe UI Variable Text\", \"Segoe UI\", Inter, system-ui, sans-serif".into(), size: 14.0, title_weight: 700, letter_spacing: 0.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BgKind {
    /// Звёзды + туманность
    #[default]
    Space,
    Gradient,
    Solid,
    /// Картинка или анимированный GIF/WebP
    Image,
    /// Видео (mp4/webm) — аппаратное декодирование, легче GIF для CPU
    Video,
    /// Размытая обложка текущего трека
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BgFit {
    #[default]
    Cover,
    Contain,
    Tile,
    Stretch,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    pub kind: BgKind,
    /// Путь к файлу или URL (для Image/Video)
    pub source: String,
    pub blur: f32,
    pub dim: f32,
    pub fit: BgFit,
    pub nebula: bool,
    pub nebula_intensity: f32,
    /// Поверх картинки/видео тоже показывать звёзды
    pub stars_over_media: bool,
}

impl Default for Background {
    fn default() -> Self {
        Self { kind: BgKind::Space, source: String::new(), blur: 0.0, dim: 0.25, fit: BgFit::Cover, nebula: true, nebula_intensity: 0.8, stars_over_media: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stars {
    pub density: f32,
    pub speed: f32,
    pub size: f32,
    pub twinkle: bool,
    pub shooting: bool,
    pub color: String,
}

impl Default for Stars {
    fn default() -> Self {
        Self { density: 1.0, speed: 1.0, size: 1.0, twinkle: true, shooting: true, color: "#ffffff".into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SidebarPos {
    /// Панель-док со значками в заголовке окна
    #[default]
    Top,
    Left,
    Right,
    /// Устаревшее значение старых тем — отображается как `Top`
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayerPos {
    #[default]
    Bottom,
    Top,
    Floating,
    /// Парящий плеер вверху, под верхней панелью.
    FloatingTop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PlayerStyle {
    #[default]
    Classic,
    Centered,
    Minimal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Side {
    Left,
    #[default]
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CoverShape {
    Square,
    #[default]
    Rounded,
    Circle,
    Vinyl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Control {
    Shuffle,
    Prev,
    Play,
    Next,
    Repeat,
}

impl Control {
    pub const ALL: [Control; 5] = [Control::Shuffle, Control::Prev, Control::Play, Control::Next, Control::Repeat];
    pub fn label(self) -> &'static str {
        match self {
            Control::Shuffle => "Перемешать",
            Control::Prev => "Назад",
            Control::Play => "Play / Pause",
            Control::Next => "Вперёд",
            Control::Repeat => "Повтор",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Extra {
    Like,
    Lyrics,
    Fullscreen,
    Queue,
    Eq,
    Volume,
}

impl Extra {
    pub const ALL: [Extra; 6] = [Extra::Like, Extra::Lyrics, Extra::Fullscreen, Extra::Queue, Extra::Eq, Extra::Volume];
    pub fn label(self) -> &'static str {
        match self {
            Extra::Like => "В избранное",
            Extra::Lyrics => "Текст сбоку",
            Extra::Fullscreen => "Текст на весь экран",
            Extra::Queue => "Очередь",
            Extra::Eq => "Эквалайзер",
            Extra::Volume => "Громкость",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub visible: bool,
}

fn entries(ids: &[&str]) -> Vec<Entry> {
    ids.iter().map(|id| Entry { id: id.to_string(), visible: true }).collect()
}

pub const NAV_ITEMS: &[(&str, &str, &str)] = &[
    ("home", "Главная", "home"),
    ("search", "Поиск", "search"),
    ("genres", "Жанры", "grid"),
    ("favorites", "Избранное", "heart"),
    ("playlists", "Плейлисты", "playlist"),
    ("history", "История", "history"),
    ("local", "Моя музыка", "folder"),
    ("soundcloud", "Мой SoundCloud", "cloud"),
];

pub const HOME_SECTIONS: &[(&str, &str)] = &[
    ("hero", "Приветствие"),
    ("continue", "Продолжить"),
    ("listening", "Время прослушивания"),
    ("recent", "Недавно слушали"),
    ("related", "Похоже на ваше"),
    ("genres", "Жанры"),
    ("favorites", "Избранное"),
    ("local", "Моя музыка"),
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub sidebar: SidebarPos,
    pub sidebar_width: f32,
    pub sidebar_compact: bool,
    pub player: PlayerPos,
    pub player_style: PlayerStyle,
    pub player_height: f32,
    pub lyrics_side: Side,
    pub cover_shape: CoverShape,
    pub density: Density,
    pub card_size: f32,
    pub content_padding: f32,
    pub controls: Vec<Control>,
    pub extras: Vec<Extra>,
    pub nav: Vec<Entry>,
    pub home: Vec<Entry>,
    pub show_logo: bool,
    pub titlebar_title: bool,
    /// Плеер в стиле Material 3: волнистая полоса прогресса, кнопка-«сквиркл», тональные кнопки.
    pub player_md3: bool,
    /// Стиль «Сцена»: заголовок с часами и плашкой трека, значки разделов справа,
    /// плеер с круглыми кнопками, пословная подсветка текста и в боковой панели.
    pub stage_style: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            sidebar: SidebarPos::Top,
            sidebar_width: 236.0,
            sidebar_compact: false,
            player: PlayerPos::Bottom,
            player_style: PlayerStyle::Classic,
            player_height: 92.0,
            lyrics_side: Side::Right,
            cover_shape: CoverShape::Rounded,
            density: Density::Comfortable,
            card_size: 176.0,
            content_padding: 28.0,
            controls: Control::ALL.to_vec(),
            extras: vec![Extra::Like, Extra::Lyrics, Extra::Fullscreen, Extra::Queue, Extra::Eq, Extra::Volume],
            nav: entries(&NAV_ITEMS.iter().map(|n| n.0).collect::<Vec<_>>()),
            home: entries(&HOME_SECTIONS.iter().map(|n| n.0).collect::<Vec<_>>()),
            show_logo: true,
            titlebar_title: true,
            player_md3: false,
            stage_style: false,
        }
    }
}

impl Layout {
    /// Новые пункты меню/разделы из обновлений приложения добавляем в конец.
    fn normalize(&mut self) {
        for (id, _, _) in NAV_ITEMS {
            if !self.nav.iter().any(|e| e.id == *id) {
                self.nav.push(Entry { id: id.to_string(), visible: true });
            }
        }
        self.nav.retain(|e| NAV_ITEMS.iter().any(|n| n.0 == e.id));
        for (id, _) in HOME_SECTIONS {
            if !self.home.iter().any(|e| e.id == *id) {
                self.home.push(Entry { id: id.to_string(), visible: true });
            }
        }
        self.home.retain(|e| HOME_SECTIONS.iter().any(|n| n.0 == e.id));
        if !self.controls.contains(&Control::Play) {
            self.controls.insert(0, Control::Play);
        }
        let mut seen = std::collections::HashSet::new();
        self.controls.retain(|c| seen.insert(*c));
        let mut seen = std::collections::HashSet::new();
        self.extras.retain(|c| seen.insert(*c));
    }
}

/// Индивидуальный стиль одного элемента интерфейса.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ElementStyle {
    pub hidden: bool,
    pub radius: Option<f32>,
    pub background: Option<String>,
    pub opacity: Option<f32>,
    pub blur: Option<f32>,
    pub padding: Option<f32>,
    pub margin: Option<f32>,
    pub font_size: Option<f32>,
    pub font_weight: Option<u16>,
    pub color: Option<String>,
    pub border_width: Option<f32>,
    pub border_color: Option<String>,
    pub glow: Option<f32>,
    pub scale: Option<f32>,
    pub offset_x: Option<f32>,
    pub offset_y: Option<f32>,
    pub order: Option<i32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub custom_css: String,
}

impl ElementStyle {
    pub fn is_empty(&self) -> bool {
        *self == ElementStyle::default()
    }

    fn write_css(&self, id: &str, out: &mut String) {
        let sel = format!("[data-el=\"{id}\"]");
        let mut d = String::new();
        // каркас нельзя скрыть: иначе окно становится пустым и прозрачным,
        // а вернуть элемент уже нечем — редактор тоже внутри окна
        if self.hidden && !is_essential(id) {
            d.push_str("display:none !important;");
        }
        if let Some(v) = self.radius {
            let _ = write!(d, "border-radius:{v}px !important;");
        }
        if let Some(v) = &self.background {
            let _ = write!(d, "background:{} !important;", sanitize_value(v));
        }
        if let Some(v) = self.opacity {
            // каркас не делаем совсем прозрачным — по той же причине, что и не скрываем
            let v = if is_essential(id) { v.max(0.2) } else { v };
            // без !important: иначе анимации появления/ухода не могут менять
            // прозрачность, и элемент (например, шапка главной) не гаснет при
            // переходе на другую вкладку
            let _ = write!(d, "opacity:{v};");
        }
        if let Some(v) = self.blur {
            let _ = write!(d, "backdrop-filter:blur({v}px) saturate(var(--saturate)) !important;");
        }
        if let Some(v) = self.padding {
            let _ = write!(d, "padding:{v}px !important;");
        }
        // у каркаса (окно, колонка страницы) размер и отступы задаёт раскладка:
        // фиксированная высота обрезала бы страницу в низком окне (настройки не
        // докручивались до конца), а отступ ломал заход страницы под заголовок
        let frame = is_essential(id);
        if let Some(v) = self.margin.filter(|_| !frame) {
            let _ = write!(d, "margin:{v}px !important;");
        }
        if let Some(v) = self.font_size {
            let _ = write!(d, "font-size:{v}px !important;");
        }
        if let Some(v) = self.font_weight {
            let _ = write!(d, "font-weight:{v} !important;");
        }
        if let Some(v) = &self.color {
            let _ = write!(d, "color:{} !important;", sanitize_value(v));
        }
        if let Some(v) = self.border_width {
            let _ = write!(d, "border-width:{v}px !important;border-style:solid !important;");
        }
        if let Some(v) = &self.border_color {
            let _ = write!(d, "border-color:{} !important;", sanitize_value(v));
        }
        if let Some(v) = self.glow {
            let _ = write!(d, "box-shadow:0 0 {:.0}px rgba(var(--accent-rgb),{:.2}) !important;", v * 42.0, (v * 0.65).min(1.0));
        }
        if let Some(v) = self.scale {
            let _ = write!(d, "scale:{v} !important;");
        }
        if self.offset_x.is_some() || self.offset_y.is_some() {
            let _ = write!(d, "translate:{}px {}px !important;", self.offset_x.unwrap_or(0.0), self.offset_y.unwrap_or(0.0));
        }
        if let Some(v) = self.order {
            let _ = write!(d, "order:{v} !important;");
        }
        if let Some(v) = self.width.filter(|_| !frame) {
            let _ = write!(d, "width:{v}px !important;min-width:0 !important;flex-basis:auto !important;");
        }
        if let Some(v) = self.height.filter(|_| !frame) {
            let _ = write!(d, "height:{v}px !important;");
        }
        if !d.is_empty() {
            let _ = writeln!(out, "{sel}{{{d}}}");
        }
        let custom = self.custom_css.trim();
        if !custom.is_empty() {
            if custom.contains('{') {
                // полноценные правила: «&» означает сам элемент
                let _ = writeln!(out, "{}", custom.replace('&', &sel));
            } else {
                let _ = writeln!(out, "{sel}{{{custom}}}");
            }
        }
    }
}

/// Не даём значению выйти за пределы объявления (`;`, `}` и `<` недопустимы).
fn sanitize_value(v: &str) -> String {
    v.chars().filter(|c| !matches!(c, ';' | '{' | '}' | '<')).collect()
}

pub struct ElementDef {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
}

macro_rules! el {
    ($id:literal, $name:literal, $group:literal) => {
        ElementDef { id: $id, name: $name, group: $group }
    };
}

/// Реестр редактируемых элементов (для инспектора и списка в настройках).
/// Элементы каркаса, без которых окно пустое: их нельзя скрыть или сделать
/// полностью прозрачными через редактор элементов.
pub const ESSENTIAL_ELEMENTS: &[&str] = &["window", "content"];

pub fn is_essential(id: &str) -> bool {
    ESSENTIAL_ELEMENTS.contains(&id)
}

pub const ELEMENTS: &[ElementDef] = &[
    el!("window", "Окно", "Каркас"),
    el!("background", "Фон", "Каркас"),
    el!("titlebar", "Заголовок окна", "Каркас"),
    el!("sidebar", "Боковая панель", "Каркас"),
    el!("content", "Область контента", "Каркас"),
    el!("logo", "Логотип", "Боковая панель"),
    el!("nav", "Меню", "Боковая панель"),
    el!("nav_item", "Пункт меню", "Боковая панель"),
    el!("sidebar_playlists", "Плейлисты в меню", "Боковая панель"),
    el!("search", "Поле поиска", "Контент"),
    el!("page_title", "Заголовок страницы", "Контент"),
    el!("section_title", "Заголовок раздела", "Контент"),
    el!("hero", "Приветствие", "Контент"),
    el!("track_card", "Карточка трека", "Контент"),
    el!("track_row", "Строка трека", "Контент"),
    el!("genre_card", "Карточка жанра", "Контент"),
    el!("chip", "Вкладки и чипы", "Контент"),
    el!("cover", "Обложки", "Контент"),
    el!("player", "Плеер", "Плеер"),
    el!("player_info", "Трек в плеере", "Плеер"),
    el!("player_cover", "Обложка в плеере", "Плеер"),
    el!("controls", "Кнопки управления", "Плеер"),
    el!("play_button", "Кнопка Play", "Плеер"),
    el!("progress", "Полоса прогресса", "Плеер"),
    el!("extras", "Дополнительные кнопки", "Плеер"),
    el!("volume", "Громкость", "Плеер"),
    el!("lyrics", "Панель текста", "Текст песни"),
    el!("lyrics_line", "Строка текста", "Текст песни"),
    el!("lyrics_active", "Активная строка", "Текст песни"),
    el!("lyrics_full", "Текст на весь экран", "Текст песни"),
    el!("queue", "Очередь", "Панели"),
    el!("eq_panel", "Эквалайзер", "Панели"),
    el!("modal", "Диалоги", "Панели"),
    el!("menu", "Контекстное меню", "Панели"),
    el!("toast", "Уведомления", "Панели"),
    el!("button", "Кнопки", "Общее"),
    el!("input", "Поля ввода", "Общее"),
];

pub fn element_name(id: &str) -> &'static str {
    ELEMENTS.iter().find(|e| e.id == id).map(|e| e.name).unwrap_or("Элемент")
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub colors: Colors,
    pub shape: Shape,
    pub glass: Glass,
    pub font: Font,
    pub background: Background,
    pub stars: Stars,
    pub layout: Layout,
    pub elements: BTreeMap<String, ElementStyle>,
    pub custom_css: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            id: "galaxy".into(),
            name: "Галактика".into(),
            dark: true,
            colors: Colors::default(),
            shape: Shape::default(),
            glass: Glass::default(),
            font: Font::default(),
            background: Background::default(),
            stars: Stars::default(),
            layout: Layout::default(),
            elements: BTreeMap::new(),
            custom_css: String::new(),
        }
    }
}

/// `#rrggbb` / `#rgb` → «r, g, b» для rgba(); всё остальное — запасной цвет.
pub fn hex_rgb(hex: &str, fallback: (u8, u8, u8)) -> (u8, u8, u8) {
    let h = hex.trim().trim_start_matches('#');
    let p = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        6 | 8 => match (p(&h[0..2]), p(&h[2..4]), p(&h[4..6])) {
            (Some(r), Some(g), Some(b)) => (r, g, b),
            _ => fallback,
        },
        3 => match (p(&h[0..1].repeat(2)), p(&h[1..2].repeat(2)), p(&h[2..3].repeat(2))) {
            (Some(r), Some(g), Some(b)) => (r, g, b),
            _ => fallback,
        },
        _ => fallback,
    }
}

fn rgb_str(c: (u8, u8, u8)) -> String {
    format!("{}, {}, {}", c.0, c.1, c.2)
}

impl Theme {
    pub fn normalized(mut self) -> Self {
        self.layout.normalize();
        self.elements.retain(|_, s| !s.is_empty());
        self
    }

    pub fn style_of(&self, id: &str) -> ElementStyle {
        self.elements.get(id).cloned().unwrap_or_default()
    }

    /// Компиляция темы в CSS.
    pub fn css(&self, s: &Settings, accent_override: Option<&str>) -> String {
        let c = &self.colors;
        let accent = accent_override.unwrap_or(&c.accent);
        let mut css = String::with_capacity(4096);
        let anim = if s.animations && !s.low_power { s.anim_speed } else { 0.0 };
        let blur = if s.low_power { (self.glass.blur * 0.4).round() } else { self.glass.blur };

        // цвет текста и значков поверх акцента: на светлом акценте — тёмный
        let (ar, ag, ab) = hex_rgb(accent, (139, 92, 246));
        let luma = 0.2126 * ar as f32 + 0.7152 * ag as f32 + 0.0722 * ab as f32;
        let on_accent = if luma > 160.0 { "#0b0b0b" } else { "#ffffff" };

        let _ = writeln!(css, ":root{{");
        let vars: [(&str, String); 48] = [
            ("--on-accent", on_accent.to_string()),
            ("--bg", c.bg.clone()),
            ("--bg-rgb", rgb_str(hex_rgb(&c.bg, (5, 3, 15)))),
            ("--surface-rgb", rgb_str(hex_rgb(&c.surface, (20, 15, 43)))),
            ("--surface2-rgb", rgb_str(hex_rgb(&c.surface2, (31, 23, 69)))),
            ("--text", c.text.clone()),
            ("--text-rgb", rgb_str(hex_rgb(&c.text, (236, 232, 255)))),
            ("--muted", c.muted.clone()),
            ("--accent", accent.to_string()),
            ("--accent-rgb", rgb_str(hex_rgb(accent, (139, 92, 246)))),
            ("--accent2", c.accent2.clone()),
            ("--accent2-rgb", rgb_str(hex_rgb(&c.accent2, (34, 211, 238)))),
            ("--glow", c.glow.clone()),
            ("--glow-rgb", rgb_str(hex_rgb(&c.glow, (167, 139, 250)))),
            ("--danger", c.danger.clone()),
            ("--border", c.border.clone()),
            ("--radius", format!("{}px", self.shape.radius)),
            ("--radius-sm", format!("{}px", self.shape.radius_small)),
            ("--radius-lg", format!("{}px", self.shape.radius_large)),
            ("--border-w", format!("{}px", self.shape.border_width)),
            ("--cover-radius", format!("{}px", self.shape.cover_radius)),
            ("--btn-radius", format!("{}px", self.shape.button_radius)),
            ("--blur", format!("{blur}px")),
            ("--glass", format!("{}", self.glass.opacity)),
            ("--saturate", format!("{}%", self.glass.saturation)),
            ("--win-alpha", format!("{}", self.glass.window_opacity)),
            ("--glow-k", format!("{}", self.glass.glow)),
            ("--font", self.font.family.clone()),
            ("--fs", format!("{}px", self.font.size)),
            ("--title-w", format!("{}", self.font.title_weight)),
            ("--ls", format!("{}px", self.font.letter_spacing)),
            ("--sidebar-w", format!("{}px", if self.layout.sidebar_compact { 76.0 } else { self.layout.sidebar_width })),
            ("--player-h", format!("{}px", self.layout.player_height)),
            ("--card", format!("{}px", self.layout.card_size)),
            ("--pad", format!("{}px", self.layout.content_padding)),
            ("--lyrics-w", format!("{}px", s.lyrics_width)),
            ("--lyrics-fs", format!("{}px", s.lyrics_font)),
            ("--anim", format!("{anim}")),
            ("--bg-blur", format!("{}px", self.background.blur)),
            ("--bg-dim", format!("{}", self.background.dim)),
            ("--nebula", format!("{}", if self.background.nebula { self.background.nebula_intensity } else { 0.0 })),
            ("--star-speed", format!("{}", self.stars.speed.max(0.01))),
            ("--star-size", format!("{}", self.stars.size)),
            ("--star-rgb", rgb_str(hex_rgb(&self.stars.color, (255, 255, 255)))),
            ("--row-h", if self.layout.density == Density::Compact { "44px".into() } else { "56px".into() }),
            ("--ui-scale", format!("{}", s.ui_scale)),
            ("--scheme", if self.dark { "dark".into() } else { "light".into() }),
            ("--track-gap", if self.layout.density == Density::Compact { "2px".into() } else { "4px".into() }),
        ];
        for (k, v) in vars {
            let _ = writeln!(css, "  {k}: {};", sanitize_value(&v));
        }
        let _ = writeln!(css, "  color-scheme: {};", if self.dark { "dark" } else { "light" });
        let _ = writeln!(css, "}}");

        for (id, st) in &self.elements {
            st.write_css(id, &mut css);
        }
        if !self.custom_css.trim().is_empty() {
            css.push_str("\n/* свой CSS темы */\n");
            css.push_str(&self.custom_css.replace("</", "<\\/"));
        }
        css
    }
}

// ───────────────────────────── встроенные темы ─────────────────────────────

fn mono_base(t: &mut Theme) {
    t.colors = Colors {
        bg: "#070707".into(),
        surface: "#111111".into(),
        surface2: "#1a1a1a".into(),
        text: "#f4f4f4".into(),
        muted: "#8b8b8b".into(),
        accent: "#ffffff".into(),
        accent2: "#bdbdbd".into(),
        glow: "#ffffff".into(),
        danger: "#ff6b6b".into(),
        border: "rgba(255,255,255,0.07)".into(),
    };
    t.shape = Shape {
        radius: 18.0,
        radius_small: 12.0,
        radius_large: 20.0,
        border_width: 1.0,
        cover_radius: 12.0,
        button_radius: 999.0,
    };
    t.layout.sidebar = SidebarPos::Left;
    t.layout.sidebar_compact = true;
    t.layout.show_logo = false;
    t.layout.stage_style = true;
    t.layout.player = PlayerPos::Bottom;
    t.layout.player_style = PlayerStyle::Classic;
    t.layout.cover_shape = CoverShape::Rounded;
    t.glass = Glass { blur: 0.0, opacity: 0.96, saturation: 100.0, window_opacity: 1.0, glow: 0.0, liquid: false };
    t.background = Background { kind: BgKind::Solid, nebula: false, dim: 0.0, ..Background::default() };
    t.stars = Stars { density: 0.0, twinkle: false, shooting: false, ..Stars::default() };
}

fn theme(id: &str, name: &str, f: impl FnOnce(&mut Theme)) -> Theme {
    let mut t = Theme { id: id.into(), name: name.into(), ..Default::default() };
    mono_base(&mut t);
    f(&mut t);
    t.normalized()
}

/// Встроенные темы, объединённые архитектурой «Монохром»: компактная левая
/// панель, карточки без визуального шума, единый заголовок «Сцена» и сдержанные
/// тональные акценты. Тема «Монохром» идёт первой и используется по умолчанию.
pub fn builtin_themes() -> Vec<Theme> {
    vec![
        theme("mono", "Монохром", |_| {}),
        theme("black", "Чёрная", |t| {
            t.colors.bg = "#000000".into();
            t.colors.surface = "#0b0b0b".into();
            t.colors.surface2 = "#141414".into();
            t.colors.border = "rgba(255,255,255,0.08)".into();
            t.glass.opacity = 0.98;
        }),
        theme("galaxy", "Галактика", |t| {
            t.colors.bg = "#07070b".into();
            t.colors.surface = "#101018".into();
            t.colors.surface2 = "#181823".into();
            t.colors.muted = "#8d8da0".into();
            t.colors.accent = "#d4d0ec".into();
            t.colors.accent2 = "#9a96b8".into();
            t.colors.glow = "#d4d0ec".into();
            t.colors.border = "rgba(220,220,245,0.07)".into();
        }),
        theme("liquid", "Жидкое стекло", |t| {
            t.colors.bg = "#08090c".into();
            t.colors.surface = "#13151b".into();
            t.colors.surface2 = "#1c1f27".into();
            t.colors.muted = "#9499a6".into();
            t.colors.accent = "#e5e7eb".into();
            t.colors.accent2 = "#9ca3af".into();
            t.colors.glow = "#ffffff".into();
            t.colors.border = "rgba(255,255,255,0.12)".into();
            t.glass = Glass { blur: 16.0, opacity: 0.78, saturation: 125.0, window_opacity: 0.96, glow: 0.0, liquid: false };
        }),
        theme("material", "Material 3", |t| {
            t.colors.bg = "#100f12".into();
            t.colors.surface = "#19181d".into();
            t.colors.surface2 = "#232128".into();
            t.colors.text = "#efedf4".into();
            t.colors.muted = "#9b96a6".into();
            t.colors.accent = "#ded7ec".into();
            t.colors.accent2 = "#aba4bc".into();
            t.colors.glow = "#ded7ec".into();
            t.colors.border = "rgba(230,225,240,0.08)".into();
            t.layout.player_md3 = true;
        }),
        theme("orion", "Туманность Ориона", |t| {
            t.colors.bg = "#0a0709".into();
            t.colors.surface = "#141013".into();
            t.colors.surface2 = "#1d161b".into();
            t.colors.text = "#f5eff3".into();
            t.colors.muted = "#9c8b95".into();
            t.colors.accent = "#ebcadb".into();
            t.colors.accent2 = "#b899aa".into();
            t.colors.glow = "#ebcadb".into();
            t.colors.border = "rgba(255,230,244,0.07)".into();
        }),
        theme("aurora", "Северное сияние", |t| {
            t.colors.bg = "#060909".into();
            t.colors.surface = "#0e1414".into();
            t.colors.surface2 = "#151f1f".into();
            t.colors.text = "#eef5f4".into();
            t.colors.muted = "#829795".into();
            t.colors.accent = "#cfe8e3".into();
            t.colors.accent2 = "#8fb3ad".into();
            t.colors.glow = "#cfe8e3".into();
            t.colors.border = "rgba(220,250,245,0.07)".into();
        }),
        theme("supernova", "Сверхновая", |t| {
            t.colors.bg = "#0a0807".into();
            t.colors.surface = "#15110f".into();
            t.colors.surface2 = "#1f1916".into();
            t.colors.text = "#f5f0ec".into();
            t.colors.muted = "#9c8d84".into();
            t.colors.accent = "#f0d6c2".into();
            t.colors.accent2 = "#bda28e".into();
            t.colors.glow = "#f0d6c2".into();
            t.colors.border = "rgba(255,235,220,0.07)".into();
        }),
        theme("synthwave", "Синтвейв", |t| {
            t.colors.bg = "#09070d".into();
            t.colors.surface = "#13101a".into();
            t.colors.surface2 = "#1c1726".into();
            t.colors.text = "#f3eff8".into();
            t.colors.muted = "#968ca8".into();
            t.colors.accent = "#f0c8d8".into();
            t.colors.accent2 = "#9cb4c9".into();
            t.colors.glow = "#f0c8d8".into();
            t.colors.border = "rgba(240,220,255,0.08)".into();
        }),
        theme("glass", "Чистое стекло", |t| {
            t.colors.bg = "#08080a".into();
            t.colors.surface = "#111116".into();
            t.colors.surface2 = "#191920".into();
            t.colors.border = "rgba(255,255,255,0.09)".into();
            t.glass = Glass { blur: 20.0, opacity: 0.72, saturation: 120.0, window_opacity: 0.92, glow: 0.0, liquid: false };
        }),
        theme("moonlight", "Лунный свет", |t| {
            t.dark = false;
            t.colors = Colors {
                bg: "#f2f3f5".into(),
                surface: "#ffffff".into(),
                surface2: "#e8eaee".into(),
                text: "#111113".into(),
                muted: "#6b6e76".into(),
                accent: "#16161a".into(),
                accent2: "#52555d".into(),
                glow: "#16161a".into(),
                danger: "#dc2626".into(),
                border: "rgba(17,17,19,0.09)".into(),
            };
            t.glass = Glass { blur: 0.0, opacity: 0.94, saturation: 100.0, window_opacity: 1.0, glow: 0.0, liquid: false };
            t.stars.color = "#16161a".into();
        }),
    ]
}

fn user_theme_path(id: &str) -> std::path::PathBuf {
    let safe: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    paths().themes.join(format!("{safe}.json"))
}

pub fn is_builtin(id: &str) -> bool {
    builtin_themes().iter().any(|t| t.id == id)
}

/// Встроенные темы (с учётом пользовательских правок) + пользовательские.
pub fn load_all() -> Vec<Theme> {
    let mut list = builtin_themes();
    if let Ok(dir) = std::fs::read_dir(&paths().themes) {
        let mut user: Vec<Theme> = dir
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| read_json::<Theme>(&e.path()))
            .map(Theme::normalized)
            .collect();
        user.sort_by(|a, b| a.name.cmp(&b.name));
        for t in user {
            if let Some(slot) = list.iter_mut().find(|b| b.id == t.id) {
                *slot = t;
            } else {
                list.push(t);
            }
        }
    }
    list
}

pub fn save(theme: &Theme) {
    if let Err(e) = write_json(&user_theme_path(&theme.id), theme) {
        tracing::warn!("не удалось сохранить тему: {e}");
    }
}

/// Для встроенной темы — откат к исходной, для пользовательской — удаление.
pub fn reset_or_delete(id: &str) {
    let _ = std::fs::remove_file(user_theme_path(id));
}

pub fn new_id(base: &str) -> String {
    let stem: String = base
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let stem = if stem.is_empty() { "theme".to_string() } else { stem };
    let existing = load_all();
    let mut n = 1;
    loop {
        let id = format!("{stem}-{n}");
        if !existing.iter().any(|t| t.id == id) {
            return id;
        }
        n += 1;
    }
}

pub fn import(path: &std::path::Path) -> Result<Theme, String> {
    let mut t: Theme = read_json(path).ok_or("файл не похож на тему Nebula")?;
    t.id = new_id(&t.name);
    let t = t.normalized();
    save(&t);
    Ok(t)
}

pub fn export(theme: &Theme, path: &std::path::Path) -> Result<(), String> {
    write_json(path, theme).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_cannot_be_hidden_or_made_invisible() {
        let st = ElementStyle { hidden: true, opacity: Some(0.0), ..Default::default() };
        let mut css = String::new();
        st.write_css("window", &mut css);
        assert!(!css.contains("display:none"), "{css}");
        assert!(css.contains("opacity:0.2"), "{css}");
        // обычные элементы по-прежнему скрываются
        let mut other = String::new();
        st.write_css("player", &mut other);
        assert!(other.contains("display:none"));
    }

    #[test]
    fn frame_ignores_size_and_margin() {
        let st = ElementStyle { height: Some(900.0), width: Some(280.0), margin: Some(0.0), radius: Some(4.0), ..Default::default() };
        let mut frame = String::new();
        st.write_css("content", &mut frame);
        assert!(!frame.contains("height") && !frame.contains("width:") && !frame.contains("margin"), "{frame}");
        assert!(frame.contains("border-radius:4px"));
        let mut other = String::new();
        st.write_css("hero", &mut other);
        assert!(other.contains("height:900px") && other.contains("margin:0px"));
    }

    #[test]
    fn css_contains_vars_and_overrides() {
        let mut t = Theme::default();
        t.elements.insert("player".into(), ElementStyle { radius: Some(4.0), custom_css: "letter-spacing: 2px".into(), ..Default::default() });
        t.elements.insert("cover".into(), ElementStyle { custom_css: "& img { filter: grayscale(1) }".into(), ..Default::default() });
        let css = t.css(&Settings::default(), None);
        assert!(css.contains("--accent: #8b5cf6;"));
        assert!(css.contains("[data-el=\"player\"]{border-radius:4px !important;}"));
        assert!(css.contains("[data-el=\"player\"]{letter-spacing: 2px}"));
        assert!(css.contains("[data-el=\"cover\"] img { filter: grayscale(1) }"));
    }

    #[test]
    fn values_cannot_break_out() {
        let st = ElementStyle { background: Some("red;} body{display:none".into()), ..Default::default() };
        let mut s = String::new();
        st.write_css("x", &mut s);
        assert!(!s.contains("body{"), "{s}");
    }

    #[test]
    fn text_on_light_accent_is_dark() {
        let black = builtin_themes().into_iter().find(|t| t.id == "black").unwrap();
        assert!(black.css(&Settings::default(), None).contains("--on-accent: #0b0b0b;"));
        assert!(Theme::default().css(&Settings::default(), None).contains("--on-accent: #ffffff;"));
        assert_eq!(Layout::default().sidebar, SidebarPos::Top);
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(hex_rgb("#8b5cf6", (0, 0, 0)), (139, 92, 246));
        assert_eq!(hex_rgb("#fff", (0, 0, 0)), (255, 255, 255));
        assert_eq!(hex_rgb("tomato", (1, 2, 3)), (1, 2, 3));
    }

    #[test]
    fn builtins_are_unique_and_valid() {
        let list = builtin_themes();
        let mut ids: Vec<_> = list.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), list.len());
        for t in &list {
            assert!(t.layout.controls.contains(&Control::Play));
            assert_eq!(t.layout.nav.len(), NAV_ITEMS.len());
            assert!(t.layout.stage_style, "{} should inherit mono stage_style", t.id);
            assert_eq!(t.layout.sidebar, SidebarPos::Left, "{} should use compact left sidebar", t.id);
        }
        assert_eq!(list[0].id, "mono");
        assert_eq!(Settings::default().theme, "mono");
    }
}
