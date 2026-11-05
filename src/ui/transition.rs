//! Плавные переходы.
//!
//! * Стек страниц: при смене раздела старая страница не исчезает мгновенно,
//!   а доигрывает анимацию ухода, пока новая въезжает. Обе — живые элементы
//!   (без копирования DOM), у каждой своя прокрутка, поэтому уходящая страница
//!   остаётся на месте, а новая открывается сверху. Направление зависит от
//!   порядка разделов в меню и глубины (жанр/плейлист/профиль — «вглубь»).
//! * Заставка при запуске: логотип собирается, буквы названия всплывают,
//!   затем заставка улетает вперёд, а интерфейс каскадом появляется на месте.
//!
//! Все анимации — только transform/opacity, длительность умножается на
//! «скорость анимаций» из настроек.

use super::actions::take_nav_back;
use super::icons::INTRO_SVG;
use super::state::*;
use crate::theme::{Entry, NAV_ITEMS};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::cmp::Ordering;
use std::time::{Duration, Instant};

/// Сколько уходящая страница остаётся в DOM (при скорости анимаций 1×).
const LEAVE_MS: f32 = 320.0;
/// Сколько показывается логотип, прежде чем появится интерфейс.
const INTRO_SHOW_MS: f32 = 1500.0;
/// Сколько длится каскадное появление интерфейса после заставки.
const INTRO_REVEAL_MS: f32 = 1250.0;

fn scaled(ms: f32, k: f32) -> Duration {
    Duration::from_millis((ms * k.clamp(0.05, 3.0)) as u64)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir {
    Fade,
    Right,
    Left,
    In,
    Out,
}

impl Dir {
    fn enter(self) -> &'static str {
        match self {
            Dir::Fade => "pe-fade",
            Dir::Right => "pe-r",
            Dir::Left => "pe-l",
            Dir::In => "pe-in",
            Dir::Out => "pe-out",
        }
    }

    fn leave(self) -> &'static str {
        match self {
            Dir::Fade => "px-fade",
            Dir::Right => "px-r",
            Dir::Left => "px-l",
            Dir::In => "px-in",
            Dir::Out => "px-out",
        }
    }
}

/// Вложенные страницы (жанр, плейлист, профиль) открываются «вглубь».
fn depth(r: &Route) -> u8 {
    match r {
        Route::Genre(_) | Route::LocalPlaylist(_) | Route::ScPlaylist(_) | Route::User(_) => 1,
        _ => 0,
    }
}

/// Место раздела в меню: от него зависит, в какую сторону едет страница.
fn order(r: &Route, nav: &[Entry]) -> usize {
    let id = r.nav_id();
    if id == "settings" {
        return usize::MAX;
    }
    nav.iter()
        .filter(|e| e.visible)
        .position(|e| e.id == id)
        .or_else(|| NAV_ITEMS.iter().position(|n| n.0 == id).map(|i| 1000 + i))
        .unwrap_or(2000)
}

fn direction(from: &Route, to: &Route, back: bool, nav: &[Entry]) -> Dir {
    let (a, b) = (depth(from), depth(to));
    match b.cmp(&a) {
        Ordering::Greater => return Dir::In,
        Ordering::Less => return Dir::Out,
        Ordering::Equal if a > 0 => return if back { Dir::Out } else { Dir::In },
        Ordering::Equal => {}
    }
    match order(to, nav).cmp(&order(from, nav)) {
        Ordering::Greater => Dir::Right,
        Ordering::Less => Dir::Left,
        Ordering::Equal => Dir::Fade,
    }
}

/// Страница в стеке: текущая или уходящая (до `until`).
#[derive(Clone, PartialEq, Debug)]
pub struct PageSlot {
    pub route: Route,
    pub key: String,
    pub current: bool,
    /// Класс анимации входа/ухода.
    pub anim: &'static str,
    until: Option<Instant>,
}

impl PageSlot {
    pub fn class(&self) -> String {
        format!("page-wrap {} {}", if self.current { "current" } else { "leaving" }, self.anim)
    }
}

/// Текущая страница плюс уходящие. Порядок элементов стабилен (новые — в
/// конец), чтобы Dioxus не переставлял узлы: перестановка сбросила бы
/// прокрутку и перезапустила анимации.
#[derive(Default)]
pub struct PageStack {
    slots: Vec<PageSlot>,
}

impl PageStack {
    /// Привести стек к маршруту. Возвращает, через сколько нужно перерисовать,
    /// чтобы убрать уходящую страницу (если переход анимирован).
    pub fn sync(&mut self, route: &Route, animate: bool, k: f32, nav: &[Entry]) -> Option<Duration> {
        let now = Instant::now();
        self.slots.retain(|s| s.until.is_none_or(|t| t > now));
        let key = route.key();
        let cur = self.slots.iter().position(|s| s.current);
        if let Some(i) = cur {
            if self.slots[i].key == key {
                // та же страница (например, другая вкладка настроек)
                if self.slots[i].route != *route {
                    self.slots[i].route = route.clone();
                }
                return None;
            }
        }
        let back = take_nav_back();
        let dir = match cur {
            Some(i) => direction(&self.slots[i].route, route, back, nav),
            None => Dir::Fade,
        };
        let leave = scaled(LEAVE_MS, k);
        if let Some(i) = cur {
            if animate {
                let s = &mut self.slots[i];
                s.current = false;
                s.anim = dir.leave();
                s.until = Some(now + leave);
            } else {
                self.slots.remove(i);
            }
        }
        let anim = if animate { dir.enter() } else { "" };
        // быстрый возврат на страницу, которая ещё уходит, — оживляем её же
        if let Some(s) = self.slots.iter_mut().find(|s| s.key == key) {
            s.current = true;
            s.until = None;
            s.anim = anim;
            s.route = route.clone();
        } else {
            self.slots.push(PageSlot { route: route.clone(), key, current: true, anim, until: None });
        }
        if !animate {
            self.slots.retain(|s| s.current);
        }
        animate.then_some(leave)
    }

    pub fn slots(&self) -> &[PageSlot] {
        &self.slots
    }
}

// ───────────────────────────────── заставка ─────────────────────────────────

/// Нужна ли заставка при запуске с такими настройками.
pub fn intro_enabled(s: &crate::settings::Settings) -> bool {
    s.intro_anim && s.animations && !s.low_power
}

/// Запустить таймер заставки (вызывается один раз при старте).
pub fn intro_start(st: AppState) {
    if *st.intro.peek() != 1 {
        return;
    }
    let k = st.settings.peek().anim_speed;
    spawn_forever(async move {
        tokio::time::sleep(scaled(INTRO_SHOW_MS, k)).await;
        intro_reveal(st);
    });
}

/// Убрать логотип и каскадом показать интерфейс (по таймеру, клику или клавише).
pub fn intro_reveal(state: AppState) {
    let mut st = state;
    if *st.intro.peek() != 1 {
        return;
    }
    st.intro.set(2);
    let k = st.settings.peek().anim_speed;
    spawn_forever(async move {
        tokio::time::sleep(scaled(INTRO_REVEAL_MS, k)).await;
        let mut st = st;
        if *st.intro.peek() == 2 {
            st.intro.set(0);
        }
    });
}

#[component]
pub fn Intro() -> Element {
    let st = app_state();
    let leaving = *st.intro.read() == 2;
    rsx! {
        div {
            class: if leaving { "intro leave" } else { "intro" },
            title: "Нажмите, чтобы пропустить",
            onclick: move |_| intro_reveal(st),
            div { class: "intro-glow" }
            div { class: "intro-core",
                div { class: "intro-logo", dangerous_inner_html: INTRO_SVG }
                div { class: "intro-word",
                    for (i , ch) in "Nebula".chars().enumerate() {
                        span { key: "{i}", style: "--i: {i}", "{ch}" }
                    }
                }
                div { class: "intro-line" }
                div { class: "intro-tag", "galactic player" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav() -> Vec<Entry> {
        ["home", "search", "genres", "favorites"].iter().map(|id| Entry { id: id.to_string(), visible: true }).collect()
    }

    #[test]
    fn direction_follows_menu_order_and_depth() {
        let n = nav();
        assert_eq!(direction(&Route::Home, &Route::Search, false, &n), Dir::Right);
        assert_eq!(direction(&Route::Favorites, &Route::Home, false, &n), Dir::Left);
        assert_eq!(direction(&Route::Genres, &Route::Genre("rock".into()), false, &n), Dir::In);
        assert_eq!(direction(&Route::Genre("rock".into()), &Route::Genres, true, &n), Dir::Out);
        assert_eq!(direction(&Route::Home, &Route::Settings(SettingsTab::Sound), false, &n), Dir::Right);
        assert_eq!(direction(&Route::Genre("a".into()), &Route::Genre("b".into()), true, &n), Dir::Out);
    }

    #[test]
    fn stack_keeps_leaving_page_and_revives_it() {
        let n = nav();
        let mut ps = PageStack::default();
        assert!(ps.sync(&Route::Home, true, 1.0, &n).is_some());
        assert_eq!(ps.slots().len(), 1);
        assert!(ps.sync(&Route::Search, true, 1.0, &n).is_some());
        let keys: Vec<_> = ps.slots().iter().map(|s| (s.key.as_str(), s.current, s.anim)).collect();
        assert_eq!(keys, vec![("home", false, "px-r"), ("search", true, "pe-r")]);
        // быстрый возврат: «home» оживает на своём месте, порядок не меняется
        ps.sync(&Route::Home, true, 1.0, &n);
        let keys: Vec<_> = ps.slots().iter().map(|s| (s.key.as_str(), s.current)).collect();
        assert_eq!(keys, vec![("home", true), ("search", false)]);
        // вкладки настроек — одна страница, без перехода
        ps.sync(&Route::Settings(SettingsTab::Sound), true, 1.0, &n);
        assert!(ps.sync(&Route::Settings(SettingsTab::Themes), true, 1.0, &n).is_none());
        assert_eq!(ps.slots().iter().filter(|s| s.current).count(), 1);
        // без анимаций уходящие страницы сразу исчезают
        ps.sync(&Route::Home, false, 1.0, &n);
        assert_eq!(ps.slots().len(), 1);
    }
}
