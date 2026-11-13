//! Корневой компонент Nebula: состояние, раскладка, глобальные обработчики
//! (клавиатура, медиаклавиши, фокус окна), фоновые задачи (статус плеера,
//! события, предзагрузка) и автосохранение настроек/тем/библиотеки.

mod actions;
mod background;
mod chrome;
mod desk_island;
pub mod display;
mod download;
mod island_sys;
mod icons;
mod import_dialog;
mod lyrics_view;
mod overlays;
mod pages;
mod player_bar;
mod sc_account;
mod spotify_card;
mod stage;
mod settings_index;
mod settings_page;
mod state;
mod theme_editor;
mod tracks;
mod transition;
mod tray_menu;
mod widgets;

use crate::audio::{PlaybackState, PlayerEvent};
use crate::discord;
use crate::library::{LibraryData, LocalIndex};
use crate::services::svc;
use crate::settings::{Settings, WindowEffect};
use crate::theme::{self, PlayerPos, Side, SidebarPos, ELEMENTS};
use actions::*;
use background::Background;
use chrome::{style_window_frame, ResizeHandles, Sidebar, TitleBar};
use dioxus::desktop::{use_asset_handler, use_global_shortcut, use_tray_icon_event_handler, use_wry_event_handler, window, HotKeyState};
use dioxus::prelude::*;
use lyrics_view::{FullscreenHost, LyricsHost};
use overlays::{ContextMenu, EditHint, ModalHost, QueuePanel, Toasts};
use pages::*;
use player_bar::{EqPanel, PlayerBar};
use serde::Deserialize;
use settings_page::SettingsPage;
pub use state::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};
use theme_editor::Inspector;
use transition::{intro_enabled, intro_reveal, intro_start, Intro, PageStack};

const BOOT_JS: &str = r#"
const send = (m) => { try { dioxus.send(m); } catch (e) {} };
const NAMES = __NAMES__;
const KEYS = new Set(["Space","ArrowLeft","ArrowRight","ArrowUp","ArrowDown","KeyL","KeyF","KeyQ","KeyM","KeyS","KeyR","Escape","Slash","F11"]);
const CTRL = new Set(["KeyF","KeyE","Comma","ArrowLeft","ArrowRight"]);
window.addEventListener("keydown", (e) => {
  const t = e.target;
  const typing = t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable);
  if (typing && e.code !== "Escape") return;
  if (e.ctrlKey ? !CTRL.has(e.code) : !KEYS.has(e.code)) return;
  if (e.altKey || e.metaKey) return;
  e.preventDefault();
  send({ k: e.code, ctrl: e.ctrlKey, shift: e.shiftKey });
}, true);
window.addEventListener("blur", () => send({ k: "__blur" }));
window.addEventListener("focus", () => send({ k: "__focus" }));
document.addEventListener("visibilitychange", () => {
  send({ k: document.hidden ? "__hidden" : "__visible" });
  document.querySelectorAll("video.bg-video").forEach(v => { try { document.hidden ? v.pause() : v.play(); } catch (e) {} });
});
const editing = () => { const r = document.querySelector(".app"); return r && r.classList.contains("edit-mode"); };
document.addEventListener("click", (e) => {
  if (!editing()) return;
  if (e.target.closest(".inspector, .edit-hint, .win-btns, .tb-edit, .modal-backdrop")) return;
  const el = e.target.closest("[data-el]");
  if (el) { e.preventDefault(); e.stopPropagation(); send({ k: "__select", v: el.getAttribute("data-el") }); }
}, true);
let hovered = null;
document.addEventListener("mouseover", (e) => {
  if (!editing()) { if (hovered) { hovered.classList.remove("el-hover"); hovered = null; } return; }
  const el = e.target.closest("[data-el]");
  if (el === hovered) return;
  if (hovered) hovered.classList.remove("el-hover");
  hovered = el && !el.closest(".inspector") ? el : null;
  if (hovered) hovered.classList.add("el-hover");
  const h = document.getElementById("edit-hover");
  if (h) h.textContent = hovered ? (NAMES[hovered.getAttribute("data-el")] || hovered.getAttribute("data-el")) : "";
}, true);
document.addEventListener("contextmenu", (e) => {
  const t = e.target;
  if (!(t.tagName === "INPUT" || t.tagName === "TEXTAREA")) e.preventDefault();
});
document.addEventListener("dragstart", (e) => { if (e.target.tagName === "IMG") e.preventDefault(); });

// «Скользящая» подсветка активного пункта: у контейнера с data-glide="<селектор
// активного>" есть дочерний .glider, он переезжает под активный элемент.
// Меряем offset* (они не учитывают transform — значок при наведении увеличен).
const glide = (root) => {
  const g = root.querySelector(":scope > .glider");
  if (!g) return;
  let a = root.querySelector(root.dataset.glide);
  if (a && root.dataset.glideIn) a = a.querySelector(root.dataset.glideIn) || a;
  let x = 0, y = 0, n = a;
  while (n && n !== root) { x += n.offsetLeft; y += n.offsetTop; n = n.offsetParent; }
  const s = root.style;
  if (!a || n !== root || !a.offsetWidth) { s.setProperty("--go", "0"); return; }
  s.setProperty("--gx", x + "px");
  s.setProperty("--gy", y + "px");
  s.setProperty("--gw", a.offsetWidth + "px");
  s.setProperty("--gh", a.offsetHeight + "px");
  s.setProperty("--go", "1");
  // первая расстановка — без анимации, дальше подсветка ездит плавно
  if (!root.hasAttribute("data-glide-ready")) requestAnimationFrame(() => requestAnimationFrame(() => root.setAttribute("data-glide-ready", "")));
};
const glideRoots = new WeakSet();
const attachGlide = (root) => {
  if (glideRoots.has(root)) return;
  glideRoots.add(root);
  const upd = () => { if (root.isConnected) glide(root); };
  new MutationObserver(upd).observe(root, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
  if (window.ResizeObserver) new ResizeObserver(upd).observe(root);
  glide(root);
};
const scanGlide = (n) => {
  if (n.nodeType !== 1) return;
  if (n.hasAttribute("data-glide")) attachGlide(n);
  n.querySelectorAll("[data-glide]").forEach(attachGlide);
};
new MutationObserver((recs) => { for (const r of recs) r.addedNodes.forEach(scanGlide); })
  .observe(document.getElementById("main") || document.body, { childList: true, subtree: true });
scanGlide(document.body);

// Плавная прокрутка колесом мыши: жёсткая пружина с критическим затуханием —
// список мягко трогается (без скачка в первый кадр) и встаёт за ~0,25 с без
// долгого «доплывания». Новый щелчок колеса сдвигает цель, скорость
// сохраняется, поэтому быстрая прокрутка идёт ровно.
// Тачпад (мелкие шаги) и Ctrl/Shift+колесо остаются системными, в экономном
// режиме и без анимаций — обычная прокрутка.
const smoothState = new WeakMap();
const OMEGA = 20; // 1/с: жёсткость пружины (больше — быстрее встаёт)
const canScrollY = (el) => {
  if (el.scrollHeight <= el.clientHeight + 1) return false;
  const oy = getComputedStyle(el).overflowY;
  return oy === "auto" || oy === "scroll";
};
const scrollerFor = (start, dy) => {
  for (let el = start; el && el !== document.body; el = el.parentElement) {
    if (el.nodeType !== 1 || !canScrollY(el)) continue;
    const s = smoothState.get(el);
    const pos = s ? s.target : el.scrollTop;
    const max = el.scrollHeight - el.clientHeight;
    // упёрлись в край — прокрутку получает внешний контейнер
    if ((dy > 0 && pos < max - 1) || (dy < 0 && pos > 1)) return el;
  }
  return null;
};
const animateScroll = (el, s) => {
  let prev = 0;
  const step = (now) => {
    // пользователь тянет ползунок или жмёт клавиши — уступаем
    if (Math.abs(el.scrollTop - s.last) > 2) { smoothState.delete(el); return; }
    const dt = prev ? Math.min(50, Math.max(0, now - prev)) : 8;
    prev = now;
    // мелкие шаги интегрирования — устойчиво при любой частоте кадров
    for (let left = dt / 1000; left > 0; left -= 1 / 240) {
      const h = Math.min(left, 1 / 240);
      s.v += (OMEGA * OMEGA * (s.target - s.pos) - 2 * OMEGA * s.v) * h;
      s.pos += s.v * h;
    }
    const done = Math.abs(s.target - s.pos) < 0.5 && Math.abs(s.v) < 20;
    if (done) { s.pos = s.target; s.v = 0; }
    // именно instant: у .page-wrap в CSS scroll-behavior: smooth, и обычное
    // присваивание запускало бы браузерную докатку поверх нашей — список прыгал
    el.scrollTo({ top: s.pos, behavior: "instant" });
    s.last = el.scrollTop;
    if (done) { smoothState.delete(el); return; }
    requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
};
window.addEventListener("wheel", (e) => {
  if (e.defaultPrevented || e.ctrlKey || e.shiftKey) return;
  if (Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
  const app = document.querySelector(".app");
  if (app && (app.classList.contains("no-anim") || app.classList.contains("low-power"))) return;
  let dy = e.deltaY;
  if (e.deltaMode === 1) dy *= 40;
  else if (e.deltaMode === 2) dy *= innerHeight;
  else if (Math.abs(dy) < 40) return; // тачпад и так плавный
  const el = scrollerFor(e.target, dy);
  if (!el) return;
  e.preventDefault();
  const max = el.scrollHeight - el.clientHeight;
  let s = smoothState.get(el);
  if (!s) {
    s = { target: el.scrollTop, last: el.scrollTop, pos: el.scrollTop, v: 0 };
    smoothState.set(el, s);
    animateScroll(el, s);
  } else if (Math.sign(dy) !== Math.sign(s.target - s.pos)) {
    // смена направления — от текущего места, без «отката» к старой цели
    s.target = s.pos;
  }
  s.target = Math.max(0, Math.min(max, s.target + dy));
}, { passive: false });
// Ползунки перемотки и громкости: заливка двигается прямо за курсором в JS,
// а значение уходит в Rust не чаще раза за кадр (перемотка — на отпускании).
// Позицию считаем по самой полосе, без поправки на «бегунок» скрытого input.
const sliderTracks = () => document.querySelectorAll(".progress, .vol-track");
const fmtMs = (ms) => { const s = Math.floor(ms / 1000); return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0"); };
let slide = null;
const slideAt = (x) => {
  const d = slide, r = d.track.getBoundingClientRect();
  const f = Math.max(0, Math.min(1, (x - r.left) / Math.max(1, r.width)));
  const min = parseFloat(d.input.min) || 0, max = parseFloat(d.input.max) || 1;
  const step = parseFloat(d.input.step) || 0;
  let v = min + f * (max - min);
  if (step > 0) v = Math.round(v / step) * step;
  d.track.style.setProperty("--drag-w", (f * 100) + "%");
  d.input.value = String(v);
  if (d.vol) {
    const pct = d.wrap && d.wrap.querySelector(".vol-pct");
    const shown = Math.abs(v - 1) < 0.025 ? 1 : v;
    if (pct) pct.textContent = Math.round(shown * 100) + "%";
  } else {
    const t = d.wrap && d.wrap.querySelector(".time");
    if (t) t.textContent = fmtMs(v);
  }
  if (!d.raf) d.raf = requestAnimationFrame(() => {
    d.raf = 0;
    d.input.dispatchEvent(new Event("input", { bubbles: true }));
  });
};
document.addEventListener("pointerdown", (e) => {
  if (e.button !== 0) return;
  for (const track of sliderTracks()) {
    const r = track.getBoundingClientRect();
    if (!r.width || e.clientX < r.left - 4 || e.clientX > r.right + 4 || e.clientY < r.top - 9 || e.clientY > r.bottom + 9) continue;
    const input = track.querySelector('input[type="range"]');
    if (!input || input.disabled) return;
    e.preventDefault();
    e.stopPropagation();
    const vol = track.classList.contains("vol-track");
    slide = { track, input, vol, wrap: track.closest(vol ? ".volume" : ".progress-wrap"), raf: 0, id: e.pointerId };
    track.classList.add("dragging");
    try { track.setPointerCapture(e.pointerId); } catch (_) {}
    slideAt(e.clientX);
    return;
  }
}, true);
document.addEventListener("pointermove", (e) => {
  if (slide && e.pointerId === slide.id) { e.preventDefault(); slideAt(e.clientX); }
}, true);
const endSlide = (e) => {
  if (!slide || (e && e.pointerId !== slide.id)) return;
  const d = slide; slide = null;
  if (d.raf) { cancelAnimationFrame(d.raf); d.raf = 0; }
  d.input.dispatchEvent(new Event("input", { bubbles: true }));
  d.input.dispatchEvent(new Event("change", { bubbles: true }));
  // заливка держится на значении курсора, пока Rust не дорисует своё
  setTimeout(() => d.track.classList.remove("dragging"), 120);
};
document.addEventListener("pointerup", endSlide, true);
document.addEventListener("pointercancel", endSlide, true);
// островок: родной ползунок, а заливку (--di-p) обновляем сразу
document.addEventListener("input", (e) => {
  const i = e.target;
  if (!(i instanceof HTMLInputElement) || !i.closest(".di-progress")) return;
  const max = parseFloat(i.max) || 1;
  i.style.setProperty("--di-p", (parseFloat(i.value) / max * 100) + "%");
}, true);
// Границы средней колонки (.content) в CSS-переменных: парящий сверху плеер
// встаёт ровно над ней и не наезжает на боковые панели.
let colEl = null;
const colRO = new ResizeObserver(() => syncCol());
const syncCol = () => {
  const app = document.querySelector(".app");
  const c = document.querySelector("main.content");
  if (!app || !c) return;
  if (c !== colEl) { if (colEl) colRO.unobserve(colEl); colEl = c; colRO.observe(c); }
  const a = app.getBoundingClientRect(), r = c.getBoundingClientRect();
  app.style.setProperty("--col-l", (r.left - a.left) + "px");
  app.style.setProperty("--col-w", r.width + "px");
  // узкая колонка — парящему сверху плееру нужна компактная раскладка
  // (data-атрибут, а не класс: список классов .app переписывает Rust)
  const col = r.width < 860 ? "tight" : r.width < 1080 ? "narrow" : "";
  if (app.dataset.col !== col) app.dataset.col = col;
};
addEventListener("resize", syncCol);
setInterval(syncCol, 1000);
syncCol();
// Автозаполнение WebView2 («Сохранённые сведения») во всех текстовых полях
// приложения не нужно: выключаем его полю при первом фокусе.
document.addEventListener("focusin", (e) => {
  const el = e.target;
  if ((el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) && !el.hasAttribute("autocomplete")) {
    el.setAttribute("autocomplete", "off");
  }
}, true);
// Интерполяция полос прогресса с реальной частотой монитора (60/120/144/240 Гц)
// без лишних перерисовок VDOM.
const syncProgState = new WeakMap();
const driveProgress = () => {
  const app = document.querySelector(".app");
  if (!app || app.classList.contains("low-power") || !app.classList.contains("is-playing")) {
    progRaf = 0;
    return;
  }
  const now = performance.now();
  for (const el of document.querySelectorAll("[data-pos][data-dur][data-play]")) {
    if (el.classList.contains("dragging")) continue;
    const rawPos = Number(el.dataset.pos) || 0;
    const dur = Number(el.dataset.dur) || 0;
    const play = el.dataset.play === "1";
    let st = syncProgState.get(el);
    if (!st || st.rawPos !== rawPos || st.dur !== dur || st.play !== play) {
      st = { rawPos, dur, play, ts: now };
      syncProgState.set(el, st);
    }
    if (!play || dur <= 0) continue;
    const cur = Math.min(dur, st.rawPos + Math.max(0, now - st.ts));
    const pct = (cur / dur * 100).toFixed(4) + "%";
    el.style.setProperty("--pp", pct);
    const fill = el.querySelector(".pr-fill");
    if (fill) fill.style.width = pct;
  }
  progRaf = requestAnimationFrame(driveProgress);
};
let progRaf = 0;
const ensureProgLoop = () => {
  const app = document.querySelector(".app");
  if (!progRaf && app && !app.classList.contains("low-power") && app.classList.contains("is-playing")) {
    progRaf = requestAnimationFrame(driveProgress);
  }
};
new MutationObserver(ensureProgLoop).observe(document.documentElement, {
  subtree: true, attributes: true, attributeFilter: ["class", "data-pos", "data-play"]
});
ensureProgLoop();
await new Promise(() => {});
"#;

#[derive(Deserialize, Debug)]
struct JsMsg {
    k: String,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    v: Option<String>,
}

fn handle_key(st: AppState, m: JsMsg) {
    // любая клавиша во время заставки просто пропускает её
    if *st.intro.peek() == 1 && !m.k.starts_with("__") {
        intro_reveal(st);
        return;
    }
    let step_s = st.settings.peek().seek_step_s;
    let vol_step = st.settings.peek().volume_step;
    match (m.k.as_str(), m.ctrl) {
        ("Space", _) => toggle_play(st),
        ("ArrowLeft", true) => prev(st),
        ("ArrowRight", true) => next(st, true),
        ("ArrowLeft", false) => seek_rel(st, -(step_s * 1000.0) as i64),
        ("ArrowRight", false) => seek_rel(st, (step_s * 1000.0) as i64),
        ("ArrowUp", _) => {
            let v = st.settings.peek().volume;
            set_volume(st, v + vol_step);
        }
        ("ArrowDown", _) => {
            let v = st.settings.peek().volume;
            set_volume(st, v - vol_step);
        }
        ("KeyM", _) => toggle_mute(st),
        ("KeyS", false) => toggle_shuffle(st),
        ("KeyR", false) => cycle_repeat(st),
        ("KeyL", false) => actions::toggle_lyrics(st),
        ("KeyQ", false) => {
            let mut s = st.settings;
            let v = !s.peek().queue_open;
            s.write().queue_open = v;
        }
        ("KeyF", true) | ("Slash", _) => {
            navigate(st, Route::Search);
            document::eval("setTimeout(() => document.getElementById('search-input')?.focus(), 60);");
        }
        ("KeyF", false) => {
            let open = !*st.fullscreen.peek();
            chrome::set_app_mode(st, open);
        }
        ("KeyE", true) => {
            let mut e = st.edit_mode;
            let v = !*e.peek();
            e.set(v);
            if !v {
                let mut s = st.selected_el;
                s.set(None);
            }
        }
        ("Comma", true) => navigate(st, Route::Settings(SettingsTab::Sound)),
        ("F11", _) => {
            chrome::toggle_fullscreen_animated(st);
        }
        ("Escape", _) => {
            let (mut menu, mut modal, mut sel, mut edit, fs, eq) = (st.menu, st.modal, st.selected_el, st.edit_mode, st.fullscreen, st.eq_open);
            if menu.peek().is_some() {
                menu.set(None);
            } else if modal.peek().is_some() {
                modal.set(None);
            } else if sel.peek().is_some() {
                sel.set(None);
            } else if *edit.peek() {
                edit.set(false);
            } else if *eq.peek() {
                player_bar::set_eq_open(st, false);
            } else if *fs.peek() {
                chrome::set_app_mode(st, false);
            } else if window().fullscreen().is_some() {
                window().set_fullscreen(false);
            }
        }
        ("__blur", _) => {
            let mut f = st.focused;
            f.set(false);
        }
        ("__focus", _) => {
            let mut f = st.focused;
            f.set(true);
        }
        ("__hidden", _) => {
            let mut h = st.hidden;
            h.set(true);
            chrome::set_window_transitions(false);
        }
        ("__visible", _) => {
            let mut h = st.hidden;
            h.set(false);
            chrome::set_window_transitions(false);
        }
        ("__select", _) => {
            if let Some(id) = m.v {
                let mut s = st.selected_el;
                s.set(Some(id));
            }
        }
        _ => {}
    }
}

/// Отложенное сохранение: пишем на диск только после паузы в изменениях.
fn use_autosave<T: Clone + PartialEq + Send + 'static>(sig: Signal<T>, delay_ms: u64, save: fn(&T)) {
    let gen = use_hook(|| Rc::new(Cell::new(0u64)));
    use_effect(move || {
        let v = sig.read().clone();
        let g = gen.get() + 1;
        gen.set(g);
        if g == 1 {
            return; // первый запуск — это загрузка, а не изменение
        }
        let gen2 = gen.clone();
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            if gen2.get() == g {
                let v2 = v.clone();
                std::thread::spawn(move || save(&v2));
            }
        });
    });
}

/// urn аккаунта SoundCloud, на который сейчас засчитывается время прослушивания.
pub fn sc_account(st: AppState) -> Option<String> {
    st.sc_user.peek().as_ref().map(|u| u.urn.clone()).filter(|u| !u.is_empty())
}

/// Вернуть окно из трея (или из островка, если он открыт).
fn show_from_tray(st: AppState) {
    let island = {
        let s = st.settings.peek();
        s.player_collapsed && s.island_desktop
    };
    if island {
        desk_island::fly_in(st);
        return;
    }
    let w = window();
    w.set_visible(true);
    w.set_minimized(false);
    w.set_focus();
}

fn apply_window_effect(effect: WindowEffect, dark: bool) {
    let w = window();
    let win: &dioxus::desktop::tao::window::Window = &w.window;
    let _ = window_vibrancy::clear_acrylic(win);
    let _ = window_vibrancy::clear_blur(win);
    let _ = window_vibrancy::clear_mica(win);
    let res = match effect {
        WindowEffect::None => Ok(()),
        WindowEffect::Mica => window_vibrancy::apply_mica(win, Some(dark)),
        WindowEffect::Acrylic => window_vibrancy::apply_acrylic(win, Some((12, 10, 26, 96))),
        WindowEffect::Blur => window_vibrancy::apply_blur(win, Some((12, 10, 26, 80))),
    };
    if let Err(e) = res {
        tracing::warn!("эффект окна {effect:?} недоступен: {e}");
    }
}

/// Содержимое страницы для маршрута.
fn page_view(route: Route) -> Element {
    match route {
        Route::Home => rsx! { Home {} },
        Route::Search => rsx! { Search {} },
        Route::Genres => rsx! { Genres {} },
        Route::Genre(g) => rsx! { Genre { name: g } },
        Route::Favorites => rsx! { Favorites {} },
        Route::History => rsx! { History {} },
        Route::Playlists => rsx! { Playlists {} },
        Route::LocalPlaylist(id) => rsx! { LocalPlaylistPage { id } },
        Route::ScPlaylist(info) => rsx! { ScPlaylistPage { info } },
        Route::User(user) => rsx! { UserPage { user } },
        Route::Local => rsx! { Local {} },
        Route::SoundCloud => rsx! { SoundCloudPage {} },
        Route::Settings(tab) => rsx! { SettingsPage { tab } },
    }
}

#[component]
pub fn App() -> Element {
    let mut st = use_context_provider(|| {
        let mut settings = Settings::load();
        // переход на новый стиль: один раз включаем тему «Монохром» (в «Темах»
        // можно вернуть прежнюю — она никуда не делась)
        // запуск Nebula — это просьба показать окно: островок с прошлого раза
        // не открываем (иначе приложение «пряталось» бы с самого старта)
        if settings.player_collapsed && settings.island_desktop {
            settings.player_collapsed = false;
        }
        if !settings.style_mono_applied {
            settings.style_mono_applied = true;
            settings.theme = "mono".into();
            settings.save();
        }
        let themes = theme::load_all();
        let theme = themes.iter().find(|t| t.id == settings.theme).cloned().unwrap_or_else(|| themes[0].clone());
        let local = LocalIndex::load().tracks();
        let intro = if intro_enabled(&settings) { 1 } else { 0 };
        display::set_low_power(settings.low_power);
        let init_hz = display::sync_app_hz(&window().window);
        AppState {
            intro: Signal::new(intro),
            player_switching: Signal::new(false),
            settings: Signal::new(settings),
            theme: Signal::new(theme),
            themes: Signal::new(themes),
            library: Signal::new(LibraryData::load()),
            local: Signal::new(local),
            local_scanning: Signal::new(false),
            route: Signal::new(Route::Home),
            back: Signal::new(Vec::new()),
            forward: Signal::new(Vec::new()),
            queue: Signal::new(Queue::default()),
            now: Signal::new(None),
            load_id: Signal::new(0),
            pstate: Signal::new(PlaybackState::Idle),
            position: Signal::new(0),
            duration: Signal::new(0),
            buffered: Signal::new(0.0),
            device: Signal::new(String::new()),
            preview: Signal::new(false),
            preloaded: Signal::new(Preloaded::default()),
            restore_pos: Signal::new(None),
            lyrics: Signal::new(LyricsState::Idle),
            lyrics_offset: Signal::new(0),
            fullscreen: Signal::new(false),
            refresh_hz: Signal::new(init_hz),
            eq_open: Signal::new(false),
            edit_mode: Signal::new(false),
            selected_el: Signal::new(None),
            toasts: Signal::new(Vec::new()),
            menu: Signal::new(None),
            modal: Signal::new(None),
            accent: Signal::new(None),
            focused: Signal::new(true),
            hidden: Signal::new(false),
            level: Signal::new(0.0),
            sc_ready: Signal::new(svc().sc.configured()),
            sc_user: Signal::new(svc().sc.user()),
            sc_login_busy: Signal::new(false),
            closing: Signal::new(false),
            reopening: Signal::new(false),
            search_query: Signal::new(String::new()),
            errors_in_row: Signal::new(0),
            related: Signal::new(None),
        }
    });

    // ── медиа (обложки, файлы фона) прямо из Rust, тем же origin ──
    use_asset_handler("media", move |req, responder| svc().media.handle(req, responder));

    // ── старт: восстановить сессию, обновить локальную библиотеку ──
    let mut opening = use_signal(|| true);
    let mut maximized = use_signal(|| false);
    let mut win_anim = use_signal(|| "");
    let win_anim_gen = use_hook(|| Rc::new(Cell::new(0u32)));
    let page_stack = use_hook(|| Rc::new(RefCell::new(PageStack::default())));
    let page_tick = use_signal(|| 0u64);
    use_hook(move || {
        restore_session(st);
        intro_start(st);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(700)).await;
            opening.set(false);
        });
        spawn(async move {
            tokio::time::sleep(Duration::from_secs(3)).await;
            if !st.settings.peek().music_folders.is_empty() || (cfg!(debug_assertions) && std::env::var("NEBULA_EXTRA_MUSIC_DIR").is_ok()) {
                pages::rescan(st, false);
            }
            let limit = st.settings.peek().image_cache_mb;
            let _ = bg(async move { tokio::task::spawn_blocking(move || crate::media::trim_cache(limit)).await }).await;
        });
    });

    // ── глобальный JS: клавиши, фокус, режим редактирования ──
    use_future(move || async move {
        let names: serde_json::Map<String, serde_json::Value> = ELEMENTS.iter().map(|e| (e.id.to_string(), serde_json::Value::String(e.name.to_string()))).collect();
        let js = BOOT_JS.replace("__NAMES__", &serde_json::Value::Object(names).to_string());
        let mut ev = document::eval(&js);
        while let Ok(msg) = ev.recv::<JsMsg>().await {
            handle_key(st, msg);
        }
    });

    // ── медиаклавиши клавиатуры (работают и в фоне) ──
    let _ = use_global_shortcut("MediaPlayPause", move |s| if s == HotKeyState::Pressed { toggle_play(st) });
    let _ = use_global_shortcut("MediaTrackNext", move |s| if s == HotKeyState::Pressed { next(st, true) });
    let _ = use_global_shortcut("MediaTrackPrevious", move |s| if s == HotKeyState::Pressed { prev(st) });
    let _ = use_global_shortcut("MediaStop", move |s| if s == HotKeyState::Pressed { svc().player.pause() });

    // ── события окна: развёрнуто ли, закрытие через Alt+F4 ──
    // (события приходят от всех окон, а островок — отдельное окно)
    let main_id = use_hook(|| window().id());
    use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if let Event::WindowEvent { event, window_id, .. } = event {
            if *window_id != main_id {
                return;
            }
            match event {
                WindowEvent::Resized(_) => {
                    let hz = display::sync_app_hz(&window().window);
                    if *st.refresh_hz.peek() != hz {
                        let mut rhz = st.refresh_hz;
                        rhz.set(hz);
                    }
                    // полный экран (F11) — тоже «развёрнуто»: без скруглённых углов и с той же анимацией
                    let m = { let w = window(); w.is_maximized() || w.fullscreen().is_some() };
                    if *maximized.peek() != m {
                        maximized.set(m);
                        let (anim, k) = {
                            let s = st.settings.peek();
                            (s.animations && !s.low_power, s.anim_speed.clamp(0.05, 3.0))
                        };
                        if anim && !*opening.peek() {
                            let g = win_anim_gen.get() + 1;
                            win_anim_gen.set(g);
                            win_anim.set(if m { "win-grow" } else { "win-shrink" });
                            let gen = win_anim_gen.clone();
                            spawn(async move {
                                tokio::time::sleep(Duration::from_millis((500.0 * k) as u64 + 40)).await;
                                if gen.get() == g {
                                    win_anim.set("");
                                }
                            });
                        }
                    }
                }
                WindowEvent::Moved(_) | WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Focused(true) => {
                    let hz = display::sync_app_hz(&window().window);
                    if *st.refresh_hz.peek() != hz {
                        let mut rhz = st.refresh_hz;
                        rhz.set(hz);
                    }
                }
                WindowEvent::CloseRequested => {
                    // с «трей вместо закрытия» Alt+F4 лишь прячет окно — островок не трогаем
                    if !st.settings.peek().close_to_tray {
                        // приложение закрывается — служебные окна тоже, иначе оно не завершится
                        desk_island::close_now();
                        tray_menu::close();
                    }
                    st.settings.peek().save();
                    st.library.peek().save();
                    save_session(st);
                }
                _ => {}
            }
        }
    });

    // ── события плеера ──
    use_future(move || async move {
        let Some(mut rx) = svc().player.take_events() else { return };
        while let Some(ev) = rx.recv().await {
            let current = *st.load_id.peek();
            match ev {
                PlayerEvent::Ended { load_id } if load_id == current => next(st, false),
                PlayerEvent::Error { load_id, message } if load_id == current => {
                    let title = st.now.peek().as_ref().map(|t| t.title.clone()).unwrap_or_default();
                    toast(st, ToastKind::Error, format!("«{title}»: {message}"));
                    let mut p = st.pstate;
                    p.set(PlaybackState::Error);
                    skip_after_error(st, load_id);
                }
                PlayerEvent::Started { load_id } if load_id == current => {
                    let mut e = st.errors_in_row;
                    e.set(0);
                }
                PlayerEvent::DeviceChanged(name) => {
                    let mut d = st.device;
                    d.set(name);
                }
                _ => {}
            }
        }
    });

    // ── опрос состояния плеера: 10 Гц во время игры, ~3 Гц в паузе ──
    use_future(move || async move {
        let mut tick: u64 = 0;
        let mut stats_load_id = 0u64;
        let mut stats_track_id = String::new();
        let mut stats_position = 0u64;
        let mut stats_pending_ms = 0u64;
        let mut stats_pending_track = None::<crate::model::Track>;
        let mut stats_flush_at = Instant::now();
        let mut rpc_key = None::<(String, bool)>;
        let mut rpc_sent_at = Instant::now() - Duration::from_secs(30);
        let mut rpc_sent_pos = 0u64;
        loop {
            let active = st.pstate.peek().is_active();
            tokio::time::sleep(Duration::from_millis(if active { 100 } else { 330 })).await;
            tick += 1;
            let snap = svc().player.snapshot();
            let (mut ps, mut pos, mut dur, mut buf) = (st.pstate, st.position, st.duration, st.buffered);
            if snap.load_id != 0 && snap.load_id == *st.load_id.peek() {
                // ошибку/загрузку UI выставляет сам, плеер лишь подтверждает
                if *ps.peek() != snap.state && !(snap.state == PlaybackState::Idle) {
                    ps.set(snap.state);
                }
                if *pos.peek() != snap.position_ms && snap.state != PlaybackState::Loading {
                    pos.set(snap.position_ms);
                }
                if snap.duration_ms > 0 && *dur.peek() != snap.duration_ms {
                    dur.set(snap.duration_ms);
                }
                let b = (snap.buffered * 100.0).round() / 100.0;
                if (*buf.peek() - b).abs() > f32::EPSILON {
                    buf.set(b);
                }

                // Позиция меняется и при перемотке, поэтому засчитываем только
                // небольшие монотонные дельты в одном и том же треке/загрузке.
                // Это не добавляет работу в аудиоколбэк и не создаёт запись на
                // диске чаще, чем раз в несколько секунд.
                if snap.load_id != stats_load_id && stats_pending_ms > 0 {
                    if let Some(old_track) = stats_pending_track.take() {
                        { let acct = sc_account(st); st.library.write().record_listening(&old_track, stats_pending_ms, acct.as_deref()); }
                    }
                    stats_pending_ms = 0;
                    stats_flush_at = Instant::now();
                }
                let track_id = st.now.peek().as_ref().map(|t| t.id.clone()).unwrap_or_default();
                if snap.state == PlaybackState::Playing {
                    if snap.load_id == stats_load_id && track_id == stats_track_id {
                        let delta = snap.position_ms.saturating_sub(stats_position);
                        if delta <= 1_500 {
                            if stats_pending_track.as_ref().is_some_and(|t| t.id != track_id) {
                                if let Some(old_track) = stats_pending_track.take() {
                                    { let acct = sc_account(st); st.library.write().record_listening(&old_track, stats_pending_ms, acct.as_deref()); }
                                }
                                stats_pending_ms = 0;
                            }
                            if stats_pending_track.is_none() {
                                stats_pending_track = st.now.peek().clone();
                            }
                            stats_pending_ms = stats_pending_ms.saturating_add(delta);
                        }
                    }
                    stats_load_id = snap.load_id;
                    stats_track_id = track_id;
                    stats_position = snap.position_ms;
                }
                if snap.state != PlaybackState::Playing && stats_pending_ms > 0 {
                    if let Some(track) = stats_pending_track.take() {
                        { let acct = sc_account(st); st.library.write().record_listening(&track, stats_pending_ms, acct.as_deref()); }
                    }
                    stats_pending_ms = 0;
                    stats_flush_at = Instant::now();
                }
                if stats_pending_ms >= 1_000 && (stats_pending_ms >= 5_000 || stats_flush_at.elapsed() >= Duration::from_secs(5)) {
                    if let Some(track) = stats_pending_track.take() {
                        { let acct = sc_account(st); st.library.write().record_listening(&track, stats_pending_ms, acct.as_deref()); }
                    }
                    stats_pending_ms = 0;
                    stats_flush_at = Instant::now();
                }

                let settings = st.settings.peek();
                let rpc_enabled = settings.discord_rpc_enabled;
                // всегда встроенное приложение Discord (поле в настройках убрано)
                let rpc_client_id = discord::DEFAULT_CLIENT_ID.to_string();
                drop(settings);
                if rpc_enabled {
                    discord::init(&rpc_client_id);
                    if let Some(track) = st.now.peek().clone() {
                        let playing = snap.state == PlaybackState::Playing;
                        let key = (track.id.clone(), playing);
                        // перемотка: позиция ушла от ожидаемой — обновить полосу времени
                        let expected = rpc_sent_pos + if playing { rpc_sent_at.elapsed().as_millis() as u64 } else { 0 };
                        let jumped = snap.position_ms.abs_diff(expected) > 2_500;
                        if rpc_key.as_ref() != Some(&key) || jumped || rpc_sent_at.elapsed() > Duration::from_secs(15) {
                            let cover = if track.is_local() { None } else { track.artwork.as_deref().map(|u| crate::model::sc_art_size(u, crate::model::ArtSize::Large)).filter(|u| u.starts_with("https://")) };
                            let url = track.permalink.clone().filter(|u| u.starts_with("https://soundcloud.com/"));
                            discord::set_activity(discord::Presence {
                                title: track.title.clone(),
                                artist: track.artist.clone(),
                                paused: !playing,
                                position_ms: snap.position_ms,
                                duration_ms: snap.duration_ms.max(track.duration_ms),
                                cover,
                                url,
                            });
                            rpc_key = Some(key);
                            rpc_sent_at = Instant::now();
                            rpc_sent_pos = snap.position_ms;
                        }
                    }
                } else if rpc_key.is_some() {
                    discord::clear();
                    rpc_key = None;
                }
            }
            let s = st.settings.peek();
            let glow = s.beat_glow && !s.low_power;
            drop(s);
            if glow && snap.state == PlaybackState::Playing {
                let lv = (svc().player.level() * 20.0).round() / 20.0;
                let mut l = st.level;
                if (*l.peek() - lv).abs() > f32::EPSILON {
                    l.set(lv);
                }
            } else if *st.level.peek() != 0.0 {
                let mut l = st.level;
                l.set(0.0);
            }
            if active && tick % 5 == 0 {
                maybe_preload(st);
            }
            if tick % 10 == 0 {
                sc_account::check_expired(st);
            }
            if tick % 120 == 0 && st.now.peek().is_some() {
                save_session(st);
            }
        }
    });

    // ── применение настроек к движку и окну (только при реальном изменении) ──
    let dsp = use_memo(move || st.settings.read().dsp());
    use_effect(move || svc().player.set_dsp(dsp()));
    let ahead = use_memo(move || st.settings.read().read_ahead_s);
    use_effect(move || svc().player.set_read_ahead(ahead() as u64 * 1000));
    let crossfade = use_memo(move || st.settings.read().crossfade_s);
    use_effect(move || svc().player.set_crossfade((crossfade() * 1000.0).round() as u64));
    let device = use_memo(move || st.settings.read().output_device.clone());
    let first_device = use_hook(|| Rc::new(Cell::new(true)));
    use_effect(move || {
        let d = device();
        if first_device.replace(false) {
            return;
        }
        svc().player.set_device(d);
    });
    let cache_mb = use_memo(move || st.settings.read().image_cache_mb);
    use_effect(move || svc().media.set_limit_mb(cache_mb()));
    let zoom = use_memo(move || st.settings.read().ui_scale);
    use_effect(move || window().set_zoom_level(zoom() as f64));
    let on_top = use_memo(move || st.settings.read().always_on_top);
    use_effect(move || window().set_always_on_top(on_top()));
    use_hook(style_window_frame);
    // островок на рабочем столе: сводка для него и команды от него
    use_future(move || async move {
        use desk_island::{IslandCmd, IslandSnap};
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            if desk_island::is_open() {
                let now = st.now.peek().clone();
                // позиция прямо из плеера и момент замера: островок сам ведёт
                // время дальше, и текст идёт точно в такт
                let playing_now = matches!(*st.pstate.peek(), PlaybackState::Playing | PlaybackState::Buffering);
                let pos = if playing_now { svc().player.snapshot().position_ms } else { *st.position.peek() };
                let pos_at = Some(Instant::now());
                // синхронный текст целиком (отправляется, только когда он сменился)
                {
                    let off = *st.lyrics_offset.peek() as i64 + st.settings.peek().lyrics_offset_ms as i64;
                    let tid = now.as_ref().map(|t| t.id.as_str()).unwrap_or("");
                    match &*st.lyrics.peek() {
                        LyricsState::Found(l) if l.is_synced() => {
                            let key = format!("{tid}:{}:{off}", l.synced.len());
                            desk_island::publish_lyrics(&key, || (l.synced.iter().map(|x| (x.time_ms, x.text.trim().to_string())).collect(), off));
                        }
                        _ => desk_island::publish_lyrics(&format!("{tid}:none"), || (Vec::new(), 0)),
                    }
                }
                let lyric = None;
                let liked = now.as_ref().is_some_and(|t| st.library.peek().is_favorite(&t.id));
                let (volume, max_volume, vol_step, muted) = {
                    let s = st.settings.peek();
                    (s.volume, s.max_volume, s.volume_step, s.muted)
                };
                let next = {
                    let q = st.queue.peek();
                    q.index.and_then(|i| q.tracks.get(i + 1)).map(|t| (t.title.clone(), t.artist.clone()))
                };
                {
                    let lib = st.library.peek();
                    let key = format!("{}:{}", lib.history.len(), lib.history.first().map(|x| x.id.as_str()).unwrap_or(""));
                    desk_island::publish_recent(&key, || lib.history.iter().take(12).cloned().collect());
                }
                desk_island::publish(IslandSnap {
                    has_track: now.is_some(),
                    track_id: now.as_ref().map(|t| t.id.clone()).unwrap_or_default(),
                    title: now.as_ref().map(|t| t.title.clone()).unwrap_or_default(),
                    artist: now.as_ref().map(|t| t.artist.clone()).unwrap_or_default(),
                    art: now.as_ref().and_then(|t| t.art(crate::model::ArtSize::Medium)),
                    playing: playing_now,
                    pos_ms: pos,
                    pos_at,
                    dur_ms: *st.duration.peek(),
                    lyric,
                    liked,
                    volume,
                    max_volume,
                    vol_step,
                    muted,
                    accent: st.accent.peek().clone(),
                    next,
                    queue_left: {
                        let q = st.queue.peek();
                        q.index.map(|i| q.tracks.len().saturating_sub(i + 1)).unwrap_or(0)
                    },
                    device: st.device.peek().clone(),
                    eq_enabled: st.settings.peek().eq_enabled,
                    eq_bands: st.settings.peek().eq_bands,
                    eq_preset: st.settings.peek().eq_preset.clone(),
                });
            }
            for cmd in desk_island::take_commands() {
                match cmd {
                    IslandCmd::TogglePlay => toggle_play(st),
                    IslandCmd::Next => next(st, true),
                    IslandCmd::Prev => prev(st),
                    IslandCmd::Seek(ms) => seek(st, ms),
                    IslandCmd::Like => {
                        if let Some(t) = st.now.peek().clone() {
                            toggle_like(st, &t);
                        }
                    }
                    IslandCmd::VolumeStep(up) => {
                        let (cur, step, muted) = {
                            let s = st.settings.peek();
                            (s.volume, s.volume_step, s.muted)
                        };
                        let base = if muted { 0.0 } else { cur };
                        set_volume(st, base + if up { step } else { -step });
                    }
                    IslandCmd::SetVolume(f) => {
                        let max = st.settings.peek().max_volume;
                        set_volume(st, f * max);
                    }
                    IslandCmd::Mute => toggle_mute(st),
                    IslandCmd::WantCorpus => {
                        // мгновенный поиск островка: очередь, избранное, своя музыка, история
                        let lib = st.library.peek();
                        let queue = st.queue.peek();
                        let local = st.local.peek();
                        let mut seen = std::collections::HashSet::new();
                        let mut tracks = Vec::new();
                        for t in queue.tracks.iter().chain(lib.favorites.iter()).chain(local.iter()).chain(lib.history.iter()) {
                            if tracks.len() >= 6000 {
                                break;
                            }
                            if seen.insert(t.id.as_str()) {
                                tracks.push(t.clone());
                            }
                        }
                        let recent = lib.recent_searches.iter().take(8).cloned().collect();
                        desk_island::publish_corpus(desk_island::Corpus { tracks, recent });
                    }
                    IslandCmd::WantPlaylists => {
                        use desk_island::PlItem;
                        let lib = st.library.peek();
                        let mut v = Vec::new();
                        if !lib.favorites.is_empty() {
                            v.push(PlItem { id: "fav".into(), name: "Избранное".into(), icon: "heart", tracks: lib.favorites.clone() });
                        }
                        for p in &lib.playlists {
                            v.push(PlItem { id: format!("pl:{}", p.id), name: p.name.clone(), icon: "playlist", tracks: p.tracks.clone() });
                        }
                        if !lib.history.is_empty() {
                            v.push(PlItem { id: "hist".into(), name: "Недавно слушали".into(), icon: "history", tracks: lib.history.iter().take(100).cloned().collect() });
                        }
                        let local = st.local.peek();
                        if !local.is_empty() {
                            v.push(PlItem { id: "local".into(), name: "Моя музыка".into(), icon: "folder", tracks: local.iter().take(3000).cloned().collect() });
                        }
                        desk_island::publish_playlists(v);
                    }
                    IslandCmd::Play(tracks, i) => play_list(st, tracks, i, "Островок · поиск"),
                    IslandCmd::EqEnabled(on) => st.settings.write().eq_enabled = on,
                    IslandCmd::EqPreset(id) => {
                        if let Some((pid, _, vals)) = crate::settings::EQ_PRESETS.iter().find(|(p, _, _)| *p == id) {
                            let mut w = st.settings.write();
                            w.eq_bands = *vals;
                            w.eq_preset = pid.to_string();
                            w.eq_enabled = true;
                        }
                    }
                    IslandCmd::EqBand(i, v) => {
                        if i < 10 {
                            let mut w = st.settings.write();
                            w.eq_bands[i] = v.clamp(-12.0, 12.0);
                            w.eq_preset = "custom".into();
                            w.eq_enabled = true;
                        }
                    }
                    IslandCmd::OpenApp => desk_island::fly_in(st),
                }
            }
        }
    });
    // островок открыт, пока плеер свёрнут (если он включён в настройках)
    let desk_island_on = use_memo(move || {
        let s = st.settings.read();
        s.player_collapsed && s.island_desktop
    });
    use_effect(move || {
        if desk_island_on() {
            desk_island::open(Duration::ZERO);
        } else {
            desk_island::close();
        }
    });
    // ── значок в трее: вернуть окно, пауза, треки, выход ──
    // ── значок в трее: левый щелчок возвращает окно, правый — своё меню ──
    let tray = use_hook(|| {
        use dioxus::desktop::trayicon::{Icon, TrayIconBuilder};
        let (rgba, w, h) = crate::app_icon_rgba();
        let mut b = TrayIconBuilder::new().with_tooltip("Nebula");
        if let Ok(icon) = Icon::from_rgba(rgba, w, h) {
            b = b.with_icon(icon);
        }
        b.build().ok()
    });
    // меню рисуем сами (tray_menu): системное почти не настраивается. Окно
    // создаём после первой отрисовки — новое дерево Dioxus посреди отрисовки
    // главного окна сбивало его, и приложение закрывалось
    use_effect(|| tray_menu::init());
    use_effect(move || {
        // значок виден, пока крестик прячет окно в трей; Alt+F4 ведёт себя так же
        let on = st.settings.read().close_to_tray;
        if let Some(t) = tray.as_ref() {
            let _ = t.set_visible(on);
        }
        window().set_close_behavior(if on { dioxus::desktop::WindowCloseBehaviour::WindowHides } else { dioxus::desktop::WindowCloseBehaviour::WindowCloses });
    });
    use_tray_icon_event_handler(move |e| {
        use dioxus::desktop::trayicon::{MouseButton, MouseButtonState, TrayIconEvent};
        if let TrayIconEvent::Click { button, button_state: MouseButtonState::Up, position, .. } = e {
            match button {
                MouseButton::Left => show_from_tray(st),
                MouseButton::Right => tray_menu::show_at(position.x, position.y),
                _ => {}
            }
        }
    });
    // сводка для меню трея и его команды
    use_future(move || async move {
        use tray_menu::{MenuSnap, TrayCmd};
        loop {
            tokio::time::sleep(Duration::from_millis(150)).await;
            {
                tray_menu::publish(MenuSnap {
                    playing: matches!(*st.pstate.peek(), PlaybackState::Playing | PlaybackState::Buffering),
                    title: st.now.peek().as_ref().map(|t| format!("{} — {}", t.title, t.artist)).unwrap_or_default(),
                });
            }
            for c in tray_menu::take_commands() {
                match c {
                    TrayCmd::Open => show_from_tray(st),
                    TrayCmd::TogglePlay => toggle_play(st),
                    TrayCmd::Next => next(st, true),
                    TrayCmd::Prev => prev(st),
                    TrayCmd::Quit => chrome::quit_app(st),
                }
            }
        }
    });
    // первый вход в аккаунт SoundCloud: время, наслушанное без входа, переходит к нему
    use_effect(move || {
        let Some(urn) = st.sc_user.read().as_ref().map(|u| u.urn.clone()).filter(|u| !u.is_empty()) else { return };
        if st.library.peek().needs_guest_adopt(&urn) {
            let mut lib = st.library;
            lib.write().adopt_guest_listening(&urn);
        }
    });
    let effect = use_memo(move || (st.settings.read().window_effect, st.theme.read().dark));
    use_effect(move || {
        let (e, dark) = effect();
        apply_window_effect(e, dark);
    });
    let dyn_accent = use_memo(move || st.settings.read().dynamic_accent);
    use_effect(move || {
        if !dyn_accent() && st.accent.peek().is_some() {
            // выключили — плавно возвращаемся к цвету темы
            fade_accent(st, None);
        }
    });

    // ── автосохранение ──
    use_autosave(st.settings, 700, |s: &Settings| s.save());
    use_autosave(st.library, 1200, |l: &LibraryData| l.save());
    let last_theme = use_hook(|| Rc::new(RefCell::new(None::<theme::Theme>)));
    use_effect(move || {
        let t = st.theme.read().clone();
        let prev = last_theme.replace(Some(t.clone()));
        // переключение темы — не правка; сохраняем только изменения той же темы
        if let Some(p) = prev {
            if p.id == t.id && p != t {
                let mut themes = st.themes;
                if let Some(slot) = themes.write().iter_mut().find(|x| x.id == t.id) {
                    *slot = t.clone();
                }
                let t2 = t.clone();
                spawn(async move {
                    tokio::time::sleep(Duration::from_millis(600)).await;
                    if *st.theme.peek() == t2 {
                        std::thread::spawn(move || theme::save(&t2));
                    }
                });
            }
        }
    });

    // ── CSS темы ──
    let css = use_memo(move || {
        let t = st.theme.read();
        let s = st.settings.read();
        let accent = st.accent.read();
        t.css(&s, accent.as_deref())
    });

    // ── раскладка ──
    let layout = st.theme.read().layout.clone();
    let has_track = st.now.read().is_some();
    let (lyrics_open, queue_open, pause_bg, player_collapsed) = {
        let s = st.settings.read();
        (s.lyrics_open && s.lyrics_enabled && has_track, s.queue_open, s.pause_bg_unfocused, s.player_collapsed)
    };
    let s = st.settings.read();
    let pstate = *st.pstate.read();
    let intro = *st.intro.read();
    let hz = *st.refresh_hz.read();
    let frame_ms = 1000.0 / hz.max(30) as f64;
    display::set_low_power(s.low_power);
    let page_anim = (s.animations && !s.low_power, s.anim_speed);
    let class = [
        "app",
        match layout.sidebar {
            SidebarPos::Left => "sb-left",
            SidebarPos::Right => "sb-right",
            SidebarPos::Top | SidebarPos::Hidden => "sb-top",
        },
        match layout.player {
            PlayerPos::Bottom => "pl-bottom",
            PlayerPos::Top => "pl-top",
            PlayerPos::Floating => "pl-float",
            PlayerPos::FloatingTop => "pl-float pl-float-top",
        },
        if !st.theme.read().dark { "light" } else { "" },
        if !s.animations || s.low_power { "no-anim" } else { "" },
        if s.low_power { "low-power" } else { "" },
        if pause_bg && (!*st.focused.read() || *st.hidden.read()) { "bg-paused" } else { "" },
        if *st.edit_mode.read() { "edit-mode" } else { "" },
        if *st.closing.read() { "closing" } else { "" },
        if *opening.read() && s.animations && !s.low_power { "opening" } else { "" },
        if *st.reopening.read() && !*st.hidden.read() { "opening" } else { "" },
        match intro {
            1 => "intro-wait",
            2 => "intro-reveal",
            _ => "",
        },
        if *maximized.read() { "maximized" } else { "" },
        *win_anim.read(),
        if player_collapsed { "player-collapsed" } else { "" },
        if st.theme.read().glass.liquid { "glass-liquid" } else { "" },
        if layout.player_md3 { "md3" } else { "" },
        if layout.stage_style { "stage-style" } else { "" },
        if s.fisheye && s.animations && !s.low_power { "fisheye" } else { "" },
        if matches!(pstate, PlaybackState::Playing | PlaybackState::Buffering) { "is-playing" } else { "is-paused" },
    ]
    .iter()
    .filter(|c| !c.is_empty())
    .copied()
    .collect::<Vec<_>>()
    .join(" ");
    drop(s);

    // ── страницы: текущая + уходящие (доигрывают анимацию поверх новой) ──
    // Смена маршрута с другим ключом создаёт новую страницу (анимация входа,
    // свежая загрузка данных при переходе жанр → жанр); старая уходит сама.
    let route = st.route.read().clone();
    let _ = page_tick.read();
    let pages: Vec<(String, String, Route)> = {
        let mut ps = page_stack.borrow_mut();
        if let Some(after) = ps.sync(&route, page_anim.0, page_anim.1, &layout.nav) {
            spawn(async move {
                tokio::time::sleep(after + Duration::from_millis(30)).await;
                let mut t = page_tick;
                let v = *t.peek();
                t.set(v.wrapping_add(1));
            });
        }
        ps.slots().iter().map(|p| (p.key.clone(), p.class(), p.route.clone())).collect()
    };
    let lyrics_left = layout.lyrics_side == Side::Left;

    rsx! {
        document::Title { "Nebula" }
        style { "{css}" }
        div { class: "{class}", "data-el": "window", style: "--hz: {hz}; --frame-ms: {frame_ms:.3}ms;",
            Background {}
            div { class: "frame",
                TitleBar {}
                div { class: "body",
                    if layout.sidebar == SidebarPos::Left {
                        Sidebar {}
                    }
                    if lyrics_left {
                        LyricsHost { open: lyrics_open, left: true }
                    }
                    main { class: "content", "data-el": "content",
                        for (key , class , route) in pages {
                            div { key: "{key}", class: "{class}", {page_view(route)} }
                        }
                        // под заголовком окна страница уходит в нарастающее размытие
                        div { class: "top-blur", "aria-hidden": "true",
                            span {} span {} span {} span {}
                        }
                    }
                    if !lyrics_left {
                        LyricsHost { open: lyrics_open, left: false }
                    }
                    if queue_open {
                        QueuePanel {}
                    }
                    if layout.sidebar == SidebarPos::Right {
                        Sidebar {}
                    }
                }
                // свёрнутый плеер живёт в островке в заголовке окна
                if !player_collapsed {
                    PlayerBar {}
                }
            }
            if intro != 0 {
                Intro {}
            }
            FullscreenHost {}
            if *st.eq_open.read() {
                EqPanel {}
            }
            ContextMenu {}
            download::DownloadDialog {}
            ModalHost {}
            Toasts {}
            if *st.edit_mode.read() {
                EditHint {}
                Inspector {}
            }
            if !*maximized.read() {
                ResizeHandles {}
            }
        }
    }
}
