//! Островок на рабочем столе: когда плеер свёрнут, главное окно прячется
//! (и с панели задач тоже), а вверху основного монитора висит маленькое окно
//! поверх всех окон Windows — чёрная «капля», вытекающая из края экрана.
//!
//! Состояния капли (между ними она плавно перетекает на пружинке):
//! - компактная: обложка, название, столбики спектра и волна под музыку;
//! - «всплывашка» на пару секунд: сменился трек или крутят громкость колёсиком;
//! - карточка при наведении: обложка, строка текста, прогресс, громкость,
//!   кнопки и «Далее» — следующий трек в очереди;
//! - поиск песен: сразу по избранному, истории и своей музыке, а по Enter —
//!   в SoundCloud или YouTube.
//!
//! Островок — отдельное окно со своим деревом Dioxus. С главным окном он
//! общается через общие структуры: «сводку» (что играет — её раз в ~150 мс
//! обновляет главное окно), очередь команд (кнопки островка — главное окно их
//! выполняет) и «корпус» треков для поиска.

use super::actions::bg;
use super::icons::Icon;
use super::AppState;
use crate::audio::EQ_FREQS;
use crate::model::{ArtSize, Track};
use crate::services::svc;
use crate::settings::{WindowEffect, EQ_PRESETS};
use dioxus::core::spawn_forever;
use dioxus::desktop::tao::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use dioxus::desktop::tao::monitor::MonitorHandle;
use dioxus::desktop::tao::window::{Window, WindowId};
use dioxus::desktop::{use_asset_handler, use_wry_event_handler, window, Config, DesktopContext, WindowBuilder};
use dioxus::prelude::*;
use parking_lot::Mutex;
use std::cell::{Cell, RefCell};
use std::iter::once;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

/// Что показывает островок.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IslandSnap {
    pub has_track: bool,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub art: Option<String>,
    pub playing: bool,
    pub pos_ms: u64,
    pub dur_ms: u64,
    pub lyric: Option<String>,
    pub liked: bool,
    /// Громкость как в настройках (1.0 = 100%, выше — усиление), её предел
    /// и шаг колёсика.
    pub volume: f32,
    pub max_volume: f32,
    pub vol_step: f32,
    pub muted: bool,
    /// Цвет обложки (динамический акцент), если он есть.
    pub accent: Option<String>,
    /// Следующий трек в очереди: название и исполнитель.
    pub next: Option<(String, String)>,
    /// Эквалайзер: включён ли, полосы (дБ) и выбранный пресет.
    pub eq_enabled: bool,
    pub eq_bands: [f32; 10],
    pub eq_preset: String,
}

/// Команды от островка главному окну.
pub enum IslandCmd {
    TogglePlay,
    Next,
    Prev,
    Seek(u64),
    Like,
    /// Колёсико: громче (`true`) или тише на шаг из настроек.
    VolumeStep(bool),
    /// Громкость долей от максимума.
    SetVolume(f32),
    Mute,
    /// Островку нужен список треков для поиска.
    WantCorpus,
    /// Играть список с указанного трека (выбор в поиске).
    Play(Vec<Track>, usize),
    /// Эквалайзер: включить/выключить, пресет, одна полоса (дБ).
    EqEnabled(bool),
    EqPreset(String),
    EqBand(usize, f32),
    /// Вернуться в Nebula: показать окно и нижний плеер.
    OpenApp,
}

/// Треки для мгновенного поиска (избранное, очередь, своя музыка, история)
/// и недавние запросы.
#[derive(Clone, Default)]
pub struct Corpus {
    pub tracks: Vec<Track>,
    pub recent: Vec<String>,
}

static SNAP: Mutex<Option<IslandSnap>> = Mutex::new(None);
static CMDS: Mutex<Vec<IslandCmd>> = Mutex::new(Vec::new());
static CORPUS: Mutex<Option<Corpus>> = Mutex::new(None);
static CORPUS_GEN: AtomicU32 = AtomicU32::new(0);
/// Островок должен доиграть уход (втянуться в край экрана) перед закрытием.
static LEAVE: AtomicBool = AtomicBool::new(false);
/// Когда островку «приземлиться»: капля из главного окна долетает до края.
static ARRIVE_AT: Mutex<Option<Instant>> = Mutex::new(None);

thread_local! {
    /// id окна островка (окна создаются и закрываются в потоке интерфейса).
    static ISLAND: RefCell<Option<WindowId>> = const { RefCell::new(None) };
    static OPENING: Cell<bool> = const { Cell::new(false) };
    /// Островок ещё нужен (сбрасывается при закрытии, пока окно создаётся).
    static WANTED: Cell<bool> = const { Cell::new(false) };
    /// Окна, которые закрыли мы сами, а не пользователь через Alt+F4.
    static ON_PURPOSE: RefCell<Vec<WindowId>> = const { RefCell::new(Vec::new()) };
    /// Поколение отложенного закрытия: повторное открытие его отменяет.
    static CLOSE_GEN: Cell<u32> = const { Cell::new(0) };
}

pub fn publish(snap: IslandSnap) {
    let mut s = SNAP.lock();
    if s.as_ref() != Some(&snap) {
        *s = Some(snap);
    }
}

pub fn publish_corpus(c: Corpus) {
    *CORPUS.lock() = Some(c);
    CORPUS_GEN.fetch_add(1, Ordering::Relaxed);
}

pub fn take_commands() -> Vec<IslandCmd> {
    std::mem::take(&mut *CMDS.lock())
}

fn send(cmd: IslandCmd) {
    CMDS.lock().push(cmd);
}

pub fn is_open() -> bool {
    ISLAND.with(|i| i.borrow().is_some()) || OPENING.get()
}

// ── геометрия окна ──

/// Состояние капли.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Compact,
    PeekTrack,
    PeekVolume,
    Card,
    Search,
    Eq,
}

const ALL_MODES: [Mode; 6] = [Mode::Compact, Mode::PeekTrack, Mode::PeekVolume, Mode::Card, Mode::Search, Mode::Eq];

impl Mode {
    /// Размер самой капли в логических пикселях — те же числа, что в ISLAND_CSS.
    fn pill(self) -> (f64, f64) {
        match self {
            Mode::Compact => (300.0, 40.0),
            Mode::PeekTrack => (364.0, 58.0),
            Mode::PeekVolume => (320.0, 46.0),
            Mode::Card => (404.0, 230.0),
            Mode::Search => (404.0, 412.0),
            Mode::Eq => (404.0, 300.0),
        }
    }

    /// Окно чуть больше капли: «ушки» по бокам, волна снизу, тень и запас
    /// на пружинку. Компактное — впритык: прозрачная часть окна тоже ловит
    /// щелчки, а под ней могут быть вкладки браузера.
    fn window(self) -> (f64, f64) {
        let (w, h) = self.pill();
        match self {
            Mode::Compact => (w + 44.0, h + 16.0),
            _ => (w + 64.0, h + 44.0),
        }
    }

    fn class(self) -> &'static str {
        match self {
            Mode::Compact => "m-compact",
            Mode::PeekTrack => "m-peek-t",
            Mode::PeekVolume => "m-peek-v",
            Mode::Card => "m-card",
            Mode::Search => "m-search",
            Mode::Eq => "m-eq",
        }
    }
}

/// Размер окна островка — под самое большое состояние. Он не меняется:
/// WebView2 догоняет новый размер окна с опозданием, и капля на кадр
/// съезжала в сторону (мигала). Видимую часть задаёт регион окна.
fn max_window() -> (f64, f64) {
    ALL_MODES.iter().map(|m| m.window()).fold((0.0, 0.0), |a, b| (a.0.max(b.0), a.1.max(b.1)))
}

/// Основной монитор (тот, что выбран основным в параметрах экрана Windows).
fn primary(win: &Window) -> Option<MonitorHandle> {
    win.primary_monitor().or_else(|| win.current_monitor())
}

/// Позиция и размер окна вверху по центру монитора, в физических пикселях
/// этого монитора (у мониторов может быть разный масштаб).
fn top_center(mon: &MonitorHandle, size: (f64, f64)) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let scale = mon.scale_factor();
    let w = (size.0 * scale).round() as u32;
    let h = (size.1 * scale).round() as u32;
    let mp = mon.position();
    let ms = mon.size();
    (PhysicalPosition::new(mp.x + (ms.width as i32 - w as i32) / 2, mp.y), PhysicalSize::new(w, h))
}

/// Сдвинуть и изменить окно одним вызовом: если делать это по очереди, между
/// ними проскакивает кадр, где капля съезжает в сторону.
fn set_rect(win: &Window, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) {
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowPos, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER};
        // SAFETY: hwnd — живое окно островка; меняем только его положение и размер
        unsafe {
            SetWindowPos(
                win.hwnd() as _,
                std::ptr::null_mut(),
                pos.x,
                pos.y,
                size.width as i32,
                size.height as i32,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }
    #[cfg(not(windows))]
    {
        win.set_outer_position(pos);
        win.set_inner_size(size);
    }
}

/// Поставить окно (постоянного размера) вверху по центру основного монитора.
fn place_window(w: &DesktopContext) {
    let win = &w.window;
    let Some(mon) = primary(win) else { return };
    let (pos, sz) = top_center(&mon, max_window());
    set_rect(win, pos, sz);
}

thread_local! {
    /// Видимая часть окна островка (логические пиксели, по центру сверху).
    static REGION: Cell<(f64, f64)> = const { Cell::new((0.0, 0.0)) };
}

/// Показать — и сделать доступной для мыши — только часть окна по центру
/// сверху. Остальное окно не рисуется и пропускает щелчки к окнам под ним.
fn place(w: &DesktopContext, size: (f64, f64)) {
    REGION.set(size);
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        use windows_sys::Win32::Graphics::Gdi::{CreateRectRgn, SetWindowRgn};
        let win = &w.window;
        let scale = win.scale_factor();
        let full = win.outer_size();
        let rw = ((size.0 * scale).round() as i32).min(full.width as i32);
        let rh = ((size.1 * scale).round() as i32).min(full.height as i32);
        let x0 = (full.width as i32 - rw) / 2;
        // SAFETY: hwnd — живое окно островка; регион после SetWindowRgn
        // принадлежит системе, удалять его самим не нужно
        unsafe {
            let rgn = CreateRectRgn(x0, 0, x0 + rw, rh);
            SetWindowRgn(win.hwnd() as _, rgn, 1);
        }
    }
    #[cfg(not(windows))]
    let _ = w;
}

/// Окну с регионом Windows рисует рамку по-старому, без DWM: при каждой смене
/// активности (щёлкнули по островку или мимо) поверх капли проступал
/// классический заголовок «как в Windows 7» с кнопками. Убираем у окна стили
/// заголовка (и не даём их вернуть) и глушим рисование рамки.
#[cfg(windows)]
fn no_classic_frame(win: &Window) {
    use dioxus::desktop::tao::platform::windows::WindowExtWindows;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE, STYLESTRUCT, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, WM_NCACTIVATE, WM_NCPAINT, WM_STYLECHANGING, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    };
    const FRAME: u32 = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;
    // недокументированные сообщения, которыми тема рисует заголовок и рамку
    const WM_NCUAHDRAWCAPTION: u32 = 0x00AE;
    const WM_NCUAHDRAWFRAME: u32 = 0x00AF;

    unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
        match msg {
            WM_NCPAINT | WM_NCUAHDRAWCAPTION | WM_NCUAHDRAWFRAME => 0,
            // активность пусть отслеживается как обычно, но без перерисовки рамки
            WM_NCACTIVATE => DefSubclassProc(hwnd, msg, wp, -1),
            WM_STYLECHANGING if wp as i32 == GWL_STYLE => {
                if let Some(s) = (lp as *mut STYLESTRUCT).as_mut() {
                    s.styleNew &= !FRAME;
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
            _ => DefSubclassProc(hwnd, msg, wp, lp),
        }
    }
    let hwnd = win.hwnd() as HWND;
    // SAFETY: hwnd — живое окно островка из этого же потока; процедура
    // подкласса — функция без состояния
    unsafe {
        SetWindowSubclass(hwnd, Some(proc), 0x4E49_534C, 0);
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(FRAME as isize));
        SetWindowPos(hwnd, std::ptr::null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
    }
}

/// Курсор сейчас над видимой частью окна (экранные координаты Windows).
fn cursor_inside(w: &DesktopContext) -> bool {
    let win = &w.window;
    let (Ok(c), Ok(p)) = (win.cursor_position(), win.outer_position()) else { return true };
    let scale = win.scale_factor();
    let (rw, rh) = REGION.get();
    let cx = p.x as f64 + win.outer_size().width as f64 / 2.0;
    let half = rw * scale / 2.0;
    c.x >= cx - half && c.x < cx + half && c.y >= p.y as f64 && c.y < p.y as f64 + rh * scale
}

/// Курсор над самой каплей (с волной), а не над прозрачными полями окна.
fn cursor_over_pill(w: &DesktopContext, m: Mode) -> bool {
    let win = &w.window;
    let (Ok(c), Ok(p)) = (win.cursor_position(), win.outer_position()) else { return true };
    let scale = win.scale_factor();
    let (pw, ph) = m.pill();
    let cx = p.x as f64 + win.outer_size().width as f64 / 2.0;
    let half = (pw / 2.0 + 14.0) * scale;
    c.x >= cx - half && c.x < cx + half && c.y >= p.y as f64 && c.y < p.y as f64 + (ph + 14.0) * scale
}

/// Где островок относительно главного окна, в CSS-пикселях его страницы:
/// туда летит капля (точка может быть и за пределами окна).
fn target_in_client(w: &DesktopContext) -> (f64, f64) {
    let win = &w.window;
    let scale = win.scale_factor();
    let size = win.inner_size().to_logical::<f64>(scale);
    let (Some(mon), Ok(ip)) = (primary(win), win.inner_position()) else { return (size.width / 2.0, -40.0) };
    let (pos, sz) = top_center(&mon, Mode::Compact.window());
    let cx = pos.x as f64 + sz.width as f64 / 2.0;
    let cy = pos.y as f64;
    ((cx - ip.x as f64) / scale, (cy - ip.y as f64) / scale)
}

// ── открытие и закрытие ──

/// Открыть островок (если ещё не открыт). Он появится не раньше `arrive_in`:
/// окно создаётся сразу (WebView2 стартует не мгновенно), а капля
/// «приземляется», когда до края долетит капля из главного окна.
pub fn open(arrive_in: Duration) {
    LEAVE.store(false, Ordering::Relaxed);
    CLOSE_GEN.with(|g| g.set(g.get() + 1));
    WANTED.set(true);
    if is_open() {
        return;
    }
    *ARRIVE_AT.lock() = Some(Instant::now() + arrive_in);
    OPENING.set(true);
    let p = crate::paths::paths();
    let (ww, wh) = max_window();
    // окно показываем, когда уже задана видимая часть, и не забираем фокус
    // у окна, в котором сейчас работают
    let mut wb = WindowBuilder::new()
        .with_title("Nebula — островок")
        .with_decorations(false)
        .with_transparent(true)
        .with_resizable(false)
        .with_always_on_top(true)
        .with_inner_size(LogicalSize::new(ww, wh))
        .with_focused(false)
        .with_visible(false);
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowBuilderExtWindows;
        wb = wb.with_skip_taskbar(true).with_undecorated_shadow(false);
    }
    // сразу создаём окно на основном мониторе, чтобы оно не мелькнуло там,
    // где сейчас главное окно
    if let Some(mon) = primary(&window().window) {
        let (pos, sz) = top_center(&mon, (ww, wh));
        wb = wb.with_position(pos).with_inner_size(sz);
    }
    let cfg = Config::new()
        .with_window(wb)
        .with_background_color((0, 0, 0, 0))
        .with_custom_head(format!("<meta name=\"color-scheme\" content=\"dark\"><style>{ISLAND_CSS}</style>"))
        .with_data_directory(&p.webview)
        .with_disable_context_menu(!cfg!(debug_assertions))
        .with_windows_browser_args(crate::webview_args());
    let pending = window().new_window(VirtualDom::new(IslandRoot), cfg);
    spawn_forever(async move {
        let ctx = pending.await;
        OPENING.set(false);
        if !WANTED.get() {
            // пока окно создавалось, островок успели закрыть
            ON_PURPOSE.with(|v| v.borrow_mut().push(ctx.id()));
            window().close_window(ctx.id());
            return;
        }
        #[cfg(windows)]
        no_classic_frame(&ctx.window);
        place_window(&ctx);
        place(&ctx, Mode::Compact.window());
        ctx.window.set_visible(true);
        ISLAND.with(|i| *i.borrow_mut() = Some(ctx.id()));
    });
}

/// Закрыть островок: капля втягивается в край экрана, потом окно закрывается.
pub fn close() {
    WANTED.set(false);
    if ISLAND.with(|i| i.borrow().is_none()) {
        return;
    }
    LEAVE.store(true, Ordering::Relaxed);
    let g = CLOSE_GEN.with(|c| {
        let v = c.get() + 1;
        c.set(v);
        v
    });
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_millis(330)).await;
        if CLOSE_GEN.with(|c| c.get()) == g {
            close_now();
        }
    });
}

/// Закрыть сразу, без анимации (выход из приложения).
pub fn close_now() {
    WANTED.set(false);
    LEAVE.store(false, Ordering::Relaxed);
    if let Some(id) = ISLAND.with(|i| i.borrow_mut().take()) {
        ON_PURPOSE.with(|v| v.borrow_mut().push(id));
        window().close_window(id);
    }
}

// ── переходы главного окна ──

/// Уход в островок: нижний плеер стекает чёрной каплей к верхнему краю
/// экрана, окно гаснет и прячется (и с панели задач), а капля «приземляется»
/// островком.
pub fn fly_out(state: AppState) {
    let mut st = state;
    let (animate, k, effect) = {
        let s = st.settings.peek();
        (s.animations && !s.low_power, s.anim_speed.clamp(0.05, 3.0), s.window_effect)
    };
    if !animate {
        st.settings.write().player_collapsed = true;
        window().set_visible(false);
        return;
    }
    let d = (760.0 * k) as u64;
    st.player_switching.set(true);
    open(Duration::from_millis(d * 62 / 100));
    // подложка Mica/Acrylic осталась бы на месте погасшего окна
    if effect != WindowEffect::None {
        super::apply_window_effect(WindowEffect::None, true);
    }
    let (tx, ty) = target_in_client(&window());
    let _ = document::eval(&drop_js(DROP_OUT_JS, tx, ty, d));
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_millis(d + 40)).await;
        let mut st = st;
        st.settings.write().player_collapsed = true;
        st.player_switching.set(false);
        window().set_visible(false);
    });
}

/// Возврат из островка: островок втягивается в край, а из-под края падает
/// капля и растекается в нижний плеер, следом проявляется окно.
pub fn fly_in(state: AppState) {
    let mut st = state;
    let (animate, k, effect) = {
        let s = st.settings.peek();
        (s.animations && !s.low_power, s.anim_speed.clamp(0.05, 3.0), s.window_effect)
    };
    let dark = st.theme.peek().dark;
    st.settings.write().player_collapsed = false;
    let w = window();
    if effect != WindowEffect::None {
        super::apply_window_effect(effect, dark);
    }
    if animate {
        let (tx, ty) = target_in_client(&w);
        let _ = document::eval(&drop_js(DROP_IN_JS, tx, ty, (680.0 * k) as u64));
    } else {
        let _ = document::eval(UNHIDE_JS);
    }
    w.set_minimized(false);
    w.set_visible(true);
    w.set_focus();
}

fn drop_js(src: &str, tx: f64, ty: f64, ms: u64) -> String {
    src.replace("__FX__", DROP_FX_JS)
        .replace("__TX__", &format!("{tx:.1}"))
        .replace("__TY__", &format!("{ty:.1}"))
        .replace("__MS__", &ms.to_string())
}

/// Общая часть эффекта капли: слой поверх окна с «липким» SVG-фильтром —
/// размытие и резкий порог по прозрачности сливают две фигуры в жидкость,
/// пока они рядом, и рвут перемычку, когда капля отрывается.
const DROP_FX_JS: &str = r#"
const tx = __TX__, ty = __TY__, D = __MS__;
document.querySelectorAll('.drop-fx').forEach(e => e.remove());
const main = document.getElementById('main');
const fx = document.createElement('div');
fx.className = 'drop-fx';
const mk = (r) => {
  const cx = r.left + r.width / 2, cy = r.top + r.height / 2;
  const x0 = Math.min(r.left, tx - 80) - 40, x1 = Math.max(r.left + r.width, tx + 80) + 40;
  const y0 = Math.min(ty, 0) - 200, y1 = r.top + r.height + 40;
  fx.style.cssText = `position:fixed;left:${x0}px;top:${y0}px;width:${x1 - x0}px;height:${y1 - y0}px;pointer-events:none;z-index:2147483647;`;
  fx.innerHTML = '<svg width="0" height="0" style="position:absolute"><filter id="drop-goo" x="0" y="0" width="100%" height="100%">'
    + '<feGaussianBlur in="SourceGraphic" stdDeviation="9" result="b"/>'
    + '<feColorMatrix in="b" mode="matrix" values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 26 -11"/></filter></svg>'
    + '<div style="position:absolute;inset:0;filter:url(#drop-goo)"><i class="pool"></i><i class="ball"></i></div>';
  document.body.appendChild(fx);
  for (const i of fx.querySelectorAll('i')) i.style.cssText = 'position:absolute;display:block;background:#000;';
  const L = (x) => (x - x0) + 'px', T = (y) => (y - y0) + 'px';
  return { cx, cy, L, T, pool: fx.querySelector('.pool'), ball: fx.querySelector('.ball') };
};
const barRect = () => {
  const bar = document.querySelector('footer.player');
  const r = bar && bar.getBoundingClientRect();
  return r && r.width > 0 ? r : { left: innerWidth / 2 - 180, top: innerHeight - 100, width: 360, height: 76 };
};
const DROP = '50% 50% 50% 50% / 42% 42% 58% 58%';
"#;

/// Капля уходит вверх: плеер чернеет и стягивается в лужицу, из неё
/// вытягивается капля и, вытягиваясь в струйку, улетает к островку.
const DROP_OUT_JS: &str = r#"
(() => {
__FX__
const r = barRect();
const { cx, cy, L, T, pool, ball } = mk(r);
pool.animate([
  { left: L(r.left), top: T(r.top), width: r.width + 'px', height: r.height + 'px', borderRadius: '18px', opacity: 0 },
  { offset: .16, opacity: 1, borderRadius: '24px' },
  { offset: .42, left: L(cx - 72), top: T(cy - 15), width: '144px', height: '30px', borderRadius: '50%', opacity: 1 },
  { offset: .7, left: L(cx - 22), top: T(cy - 4), width: '44px', height: '8px', borderRadius: '50%', opacity: 1 },
  { left: L(cx), top: T(cy), width: '0px', height: '0px', borderRadius: '50%', opacity: 0 }
], { duration: D, easing: 'cubic-bezier(.5,0,.3,1)', fill: 'forwards' });
ball.animate([
  { left: L(cx), top: T(cy), width: '0px', height: '0px', borderRadius: '50%' },
  { offset: .22, left: L(cx - 12), top: T(cy - 16), width: '24px', height: '24px', borderRadius: '50%' },
  { offset: .46, left: L(cx - 27), top: T(cy - 66), width: '54px', height: '58px', borderRadius: DROP },
  { offset: .64, left: L(cx + (tx - cx) * .22 - 22), top: T(cy - 120 + (ty - cy) * .18), width: '44px', height: '66px', borderRadius: DROP },
  { left: L(tx - 13), top: T(ty - 150), width: '26px', height: '104px', borderRadius: DROP }
], { duration: D, easing: 'cubic-bezier(.55,0,.25,1)', fill: 'forwards' });
main.animate([
  { opacity: 1, transform: 'scale(1)' },
  { opacity: 0, transform: 'scale(.965)' }
], { duration: D * .5, delay: D * .1, easing: 'cubic-bezier(.4,0,.7,1)', fill: 'forwards' });
setTimeout(() => fx.remove(), D + 400);
})();
"#;

/// Капля падает сверху и растекается в нижний плеер, следом проявляется окно.
const DROP_IN_JS: &str = r#"
(() => {
__FX__
main.getAnimations().forEach(a => a.cancel());
main.style.opacity = '0';
requestAnimationFrame(() => requestAnimationFrame(() => {
  const r = barRect();
  const { cx, cy, L, T, pool, ball } = mk(r);
  ball.animate([
    { left: L(tx - 12), top: T(ty - 140), width: '24px', height: '96px', borderRadius: DROP },
    { offset: .46, left: L(cx - 24), top: T(cy - 46), width: '48px', height: '60px', borderRadius: DROP },
    { offset: .6, left: L(cx - 30), top: T(cy - 26), width: '60px', height: '50px', borderRadius: '50%' },
    { left: L(cx), top: T(cy), width: '0px', height: '0px', borderRadius: '50%' }
  ], { duration: D, easing: 'cubic-bezier(.45,0,.2,1)', fill: 'forwards' });
  pool.animate([
    { left: L(cx), top: T(cy), width: '0px', height: '0px', borderRadius: '50%', opacity: 1 },
    { offset: .5, left: L(cx), top: T(cy), width: '0px', height: '0px', borderRadius: '50%', opacity: 1 },
    { offset: .7, left: L(cx - 90), top: T(cy - 16), width: '180px', height: '32px', borderRadius: '50%', opacity: 1 },
    { offset: .86, left: L(r.left), top: T(r.top), width: r.width + 'px', height: r.height + 'px', borderRadius: '22px', opacity: 1 },
    { left: L(r.left), top: T(r.top), width: r.width + 'px', height: r.height + 'px', borderRadius: '18px', opacity: 0 }
  ], { duration: D, easing: 'cubic-bezier(.4,0,.2,1)', fill: 'forwards' });
  main.style.opacity = '';
  main.animate([
    { opacity: 0, transform: 'scale(.97)' },
    { offset: .55, opacity: 0, transform: 'scale(.97)' },
    { opacity: 1, transform: 'scale(1)' }
  ], { duration: D, easing: 'cubic-bezier(.3,.7,.4,1)' });
  setTimeout(() => fx.remove(), D + 80);
}));
})();
"#;

/// Показать окно без анимации (анимации выключены).
const UNHIDE_JS: &str = r#"
document.querySelectorAll('.drop-fx').forEach(e => e.remove());
const m = document.getElementById('main');
m.getAnimations().forEach(a => a.cancel());
m.style.opacity = '';
"#;

// ── окно островка ──

/// Появление и уход окна островка.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Окно создано, но капля ещё не долетела.
    Wait,
    /// Капля «приземлилась» и растекается в островок.
    Arrive,
    Live,
    /// Втягивается в край перед закрытием.
    Leave,
}

const WAVE_JS: &str = r#"
// волнистый нижний край, столбики и общий уровень — по спектру (~30 раз в секунду)
const N = 48;
let phase = 0, lvlS = 0;
while (true) {
  const v = await dioxus.recv();
  const isl = document.querySelector(".isl");
  const lvl = v.reduce((a, b) => a + b, 0) / (v.length || 1);
  lvlS += (lvl - lvlS) * 0.35;
  if (isl) isl.style.setProperty("--lvl", Math.min(1, lvlS * 1.8).toFixed(3));
  for (const set of document.querySelectorAll(".isl-bars")) {
    const bars = set.children;
    for (let i = 0; i < bars.length; i++) {
      const k = Math.floor((i + 1) * v.length / (bars.length + 2));
      bars[i].style.setProperty("--h", (v[k] || 0).toFixed(3));
    }
  }
  const path = document.getElementById("isl-wave");
  if (!path) continue;
  const svg = path.ownerSVGElement;
  const w = svg.clientWidth || 300;
  phase += 0.16 + lvl * 0.5;
  let d = `M 0 0 L 0 2`;
  for (let i = 0; i <= N; i++) {
    const x = (i / N) * w;
    const band = v[Math.floor((i / N) * (v.length - 1))] || 0;
    const edge = Math.sin(Math.PI * i / N); // к краям волна затухает
    const y = 2 + edge * (band * 7 + 1.2) * (0.6 + 0.4 * Math.sin(i * 0.7 + phase));
    d += ` L ${x.toFixed(1)} ${Math.max(1, y).toFixed(2)}`;
  }
  d += ` L ${w} 0 Z`;
  path.setAttribute("d", d);
}
"#;

fn fmt(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Полосы, которые тянут мышью: перемотка, громкость и полосы эквалайзера.
/// Заливка и ползунок идут за курсором сразу (в странице), а наружу уходит
/// доля 0..1: громкость и эквалайзер — на ходу, перемотка — при отпускании.
/// Доля считается по настоящему прямоугольнику полосы на экране, поэтому
/// совпадает с курсором даже во время анимации карточки.
const SCRUB_JS: &str = r#"
const fmt = (ms) => { const s = Math.floor(ms / 1000); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };
const frac = (el, e) => {
  const r = el.getBoundingClientRect();
  const f = el.dataset.axis === 'y' ? 1 - (e.clientY - r.top) / r.height : (e.clientX - r.left) / r.width;
  return Math.min(1, Math.max(0, f));
};
const show = (el, f) => {
  if (el.dataset.scrub === 'eq') return;
  el.classList.add('drag');
  el.style.setProperty('--drag', f.toFixed(4));
  const tip = el.querySelector('.c-tip');
  if (tip) tip.textContent = el.dataset.dur ? fmt(f * +el.dataset.dur) : Math.round(f * +(el.dataset.max || 1) * 100) + '%';
};
let drag = null;
const out = (kind, f, done, i) => dioxus.send([kind, f, done ? 1 : 0, i]);
document.addEventListener('pointerdown', (e) => {
  const el = e.target.closest('[data-scrub]');
  if (!el || e.button !== 0) return;
  e.preventDefault();
  try { el.setPointerCapture(e.pointerId); } catch (_) {}
  clearTimeout(el._t);
  drag = { el, kind: el.dataset.scrub, i: +(el.dataset.i || 0), last: 0 };
  const f = frac(el, e);
  show(el, f);
  if (drag.kind !== 'seek') out(drag.kind, f, false, drag.i);
});
document.addEventListener('pointermove', (e) => {
  if (!drag) return;
  const f = frac(drag.el, e);
  show(drag.el, f);
  const now = performance.now();
  if (drag.kind !== 'seek' && now - drag.last > 35) { drag.last = now; out(drag.kind, f, false, drag.i); }
});
const end = (e) => {
  if (!drag) return;
  const { el, kind, i } = drag;
  drag = null;
  const f = frac(el, e);
  show(el, f);
  out(kind, f, true, i);
  // пока плеер перематывается и приходит новая позиция, заливка не отпрыгивает назад
  el._t = setTimeout(() => el.classList.remove('drag'), 800);
};
document.addEventListener('pointerup', end);
document.addEventListener('pointercancel', end);
// пресеты эквалайзера листаются колёсиком вбок
document.addEventListener('wheel', (e) => {
  const p = e.target.closest('.e-presets');
  if (p) { p.scrollLeft += e.deltaY || e.deltaX; e.preventDefault(); }
}, { passive: false });
await new Promise(() => {});
"#;

/// Корень окна островка.
#[component]
fn IslandRoot() -> Element {
    // обложки и файлы — через тот же кэш /media, что и в главном окне
    use_asset_handler("media", move |req, responder| svc().media.handle(req, responder));
    let my_id = use_hook(|| window().id());
    let mut snap = use_signal(IslandSnap::default);
    let mut mode = use_signal(|| Mode::Compact);
    let mut phase = use_signal(|| Phase::Wait);
    let mut mode_gen = use_signal(|| 0u32);
    let mut hover_gen = use_signal(|| 0u32);
    let mut peek_gen = use_signal(|| 0u32);
    let mut vol_override = use_signal(|| None::<f32>);
    let mut corpus_gen = use_signal(|| 0u32);
    let mut focus_lost = use_signal(|| false);
    // поиск
    let mut query = use_signal(String::new);
    let mut src = use_signal(|| "sc");
    let mut online = use_signal(|| None::<(String, Result<Vec<Track>, String>)>);
    let mut searching = use_signal(|| None::<String>);
    // эквалайзер, пока его крутят здесь: (включён, полосы, пресет) — сводка
    // из главного окна приходит с запаздыванием и дёргала бы ползунки
    let mut eq_local = use_signal(|| None::<(bool, [f32; 10], String)>);

    use_hook(|| {
        super::pages::VISUALIZERS.fetch_add(1, Ordering::Relaxed);
        svc().player.set_visualizer(true);
    });
    use_drop(move || {
        if super::pages::VISUALIZERS.fetch_sub(1, Ordering::Relaxed) <= 1 {
            svc().player.set_visualizer(false);
        }
        ISLAND.with(|i| {
            let mut b = i.borrow_mut();
            if *b == Some(my_id) {
                *b = None;
            }
        });
        let purpose = ON_PURPOSE.with(|v| {
            let mut v = v.borrow_mut();
            let had = v.contains(&my_id);
            v.retain(|x| *x != my_id);
            had
        });
        // окно закрыли не мы (Alt+F4) — без островка спрятанное главное окно
        // было бы не вернуть
        if !purpose {
            send(IslandCmd::OpenApp);
        }
    });

    // Перейти в другое состояние: видимая часть окна сразу растёт до большей
    // из двух капель, капля перетекает на пружинке, а после анимации видимая
    // часть ужимается по новой капле.
    let mut go = move |m: Mode| {
        let cur = *mode.peek();
        if cur == m {
            return;
        }
        let g = *mode_gen.peek() + 1;
        mode_gen.set(g);
        let (a, b) = (cur.window(), m.window());
        place(&window(), (a.0.max(b.0), a.1.max(b.1)));
        mode.set(m);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(620)).await;
            if *mode_gen.peek() == g {
                place(&window(), m.window());
            }
        });
    };
    // «всплывашка» на время
    let mut peek = move |m: Mode, ms: u64| {
        let g = *peek_gen.peek() + 1;
        peek_gen.set(g);
        go(m);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            if *peek_gen.peek() == g && *mode.peek() == m {
                go(Mode::Compact);
            }
        });
    };

    // приземление: ждём, пока капля из главного окна долетит до края экрана
    use_future(move || async move {
        let at = ARRIVE_AT.lock().unwrap_or_else(Instant::now);
        let now = Instant::now();
        if at > now {
            tokio::time::sleep(at - now).await;
        }
        phase.set(Phase::Arrive);
        tokio::time::sleep(Duration::from_millis(820)).await;
        if *phase.peek() == Phase::Arrive {
            phase.set(Phase::Live);
        }
    });
    // сводка из главного окна, уход и корпус для поиска
    use_future(move || async move {
        let mut tick = 0u32;
        loop {
            let leave = LEAVE.load(Ordering::Relaxed);
            let ph = *phase.peek();
            if leave && ph != Phase::Leave {
                phase.set(Phase::Leave);
            } else if !leave && ph == Phase::Leave {
                // островок снова нужен, пока он втягивался
                phase.set(Phase::Live);
            }
            let cg = CORPUS_GEN.load(Ordering::Relaxed);
            if *corpus_gen.peek() != cg {
                corpus_gen.set(cg);
            }
            if tick % 3 == 0 {
                if let Some(s) = SNAP.lock().clone() {
                    let old = snap.peek().clone();
                    if old != s {
                        let changed = !old.track_id.is_empty() && old.track_id != s.track_id;
                        if vol_override.peek().is_some_and(|v| (v - s.volume).abs() < 0.002) {
                            vol_override.set(None);
                        }
                        // главное окно уже применило эквалайзер — своя копия не нужна
                        if eq_local.peek().as_ref().is_some_and(|(on, b, p)| *on == s.eq_enabled && *b == s.eq_bands && *p == s.eq_preset) && *mode.peek() != Mode::Eq {
                            eq_local.set(None);
                        }
                        snap.set(s);
                        if changed && *mode.peek() == Mode::Compact && *phase.peek() == Phase::Live {
                            peek(Mode::PeekTrack, 2800);
                        }
                    }
                }
            }
            tick = tick.wrapping_add(1);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    // перетаскивание полос (перемотка, громкость, эквалайзер)
    use_future(move || async move {
        let mut ev = document::eval(SCRUB_JS);
        while let Ok((kind, f, done, i)) = ev.recv::<(String, f64, u8, usize)>().await {
            match kind.as_str() {
                "seek" if done == 1 => {
                    let dur = snap.peek().dur_ms;
                    send(IslandCmd::Seek((f * dur as f64) as u64));
                }
                "vol" => {
                    let max = snap.peek().max_volume.max(0.01);
                    vol_override.set(Some(f as f32 * max));
                    send(IslandCmd::SetVolume(f as f32));
                }
                "eq" if i < 10 => {
                    let v = ((f * 24.0 - 12.0) * 2.0).round() as f32 / 2.0;
                    let s = snap.peek();
                    let (_, mut bands, _) = eq_local.peek().clone().unwrap_or((s.eq_enabled, s.eq_bands, s.eq_preset.clone()));
                    drop(s);
                    if bands[i] != v || done == 1 {
                        bands[i] = v;
                        eq_local.set(Some((true, bands, "custom".into())));
                        send(IslandCmd::EqBand(i, v));
                    }
                }
                _ => {}
            }
        }
    });
    // спектр для волны и столбиков
    use_future(|| async move {
        let ev = document::eval(WAVE_JS);
        loop {
            tokio::time::sleep(Duration::from_millis(33)).await;
            let v = svc().player.spectrum();
            let vals: Vec<f32> = v.iter().map(|x| (x * 1000.0).round() / 1000.0).collect();
            let _ = ev.send(vals);
        }
    });
    // Карточка сворачивается, когда курсор ушёл с капли. mouseleave приходит
    // не всегда (после смены размера окна WebView2 «помнит» мышь внутри),
    // поэтому положение курсора ещё и проверяем сами.
    use_future(move || async move {
        let mut outside_since: Option<Instant> = None;
        loop {
            tokio::time::sleep(Duration::from_millis(120)).await;
            let m = *mode.peek();
            // эквалайзер ждёт дольше: ползунок могли увести за край, пока тянут
            let wait = match m {
                Mode::Card => 420,
                Mode::Eq => 1100,
                _ => 0,
            };
            if wait == 0 || cursor_over_pill(&window(), m) {
                outside_since = None;
                continue;
            }
            let t = *outside_since.get_or_insert_with(Instant::now);
            if t.elapsed() >= Duration::from_millis(wait) {
                outside_since = None;
                go(Mode::Compact);
            }
        }
    });
    // щёлкнули мимо островка, пока открыт поиск, — сворачиваемся
    use_wry_event_handler(move |event, _| {
        use dioxus::desktop::tao::event::{Event, WindowEvent};
        if let Event::WindowEvent { window_id, event: WindowEvent::Focused(false), .. } = event {
            if *window_id == my_id {
                focus_lost.set(true);
            }
        }
    });
    use_effect(move || {
        if focus_lost() {
            focus_lost.set(false);
            if *mode.peek() == Mode::Search && !cursor_inside(&window()) {
                go(Mode::Compact);
            }
        }
    });

    // раскрываем только если курсор и правда над окном: когда окно только
    // появилось, WebView2 присылает ложный mouseenter
    let mut open_card = move || {
        let m = *mode.peek();
        if matches!(m, Mode::Compact | Mode::PeekTrack | Mode::PeekVolume) && *phase.peek() != Phase::Leave && cursor_inside(&window()) {
            go(Mode::Card);
        }
    };
    let enter = move |_| {
        // ложный mouseenter (WebView2 повторяет его после смены размера окна)
        // не должен отменять сворачивание
        if !cursor_inside(&window()) {
            return;
        }
        let g = hover_gen() + 1;
        hover_gen.set(g);
        open_card();
    };
    // если ложный mouseenter был отброшен, настоящий уже не придёт —
    // раскрываем по первому движению мыши
    let moved = move |_| open_card();
    let leave = move |_| {
        let g = hover_gen() + 1;
        hover_gen.set(g);
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(380)).await;
            // в поиске не сворачиваемся: мышь могли увести, пока печатают
            if hover_gen() == g && *mode.peek() == Mode::Card {
                go(Mode::Compact);
            }
        });
    };
    let wheel = move |e: WheelEvent| {
        let m = *mode.peek();
        if matches!(m, Mode::Search | Mode::Eq) {
            return;
        }
        let up = e.delta().strip_units().y < 0.0;
        send(IslandCmd::VolumeStep(up));
        // отвечаем сразу, не дожидаясь сводки
        let (cur, step, max) = {
            let s = snap.peek();
            (vol_override.peek().unwrap_or(if s.muted { 0.0 } else { s.volume }), s.vol_step.max(0.01), s.max_volume.max(0.01))
        };
        vol_override.set(Some((cur + if up { step } else { -step }).clamp(0.0, max)));
        if m != Mode::Card {
            peek(Mode::PeekVolume, 1500);
        }
    };

    let mut run_search = move || {
        let q = query.peek().trim().to_string();
        if q.is_empty() {
            return;
        }
        let source = *src.peek();
        let key = format!("{source}:{q}");
        searching.set(Some(key.clone()));
        spawn(async move {
            let q2 = q.clone();
            let res = bg(async move {
                if source == "yt" {
                    svc().yt.search(&q2).await.map(|p| p.items)
                } else {
                    svc().sc.search_tracks(&q2).await.map(|p| p.items)
                }
            })
            .await
            .unwrap_or_else(|| Err("Поиск прервался".into()));
            if searching.peek().as_deref() == Some(key.as_str()) {
                online.set(Some((key, res)));
                searching.set(None);
            }
        });
    };
    let mut open_search = move || {
        send(IslandCmd::WantCorpus);
        go(Mode::Search);
        window().set_focus();
        spawn(async move {
            tokio::time::sleep(Duration::from_millis(90)).await;
            let _ = document::eval("const q = document.querySelector('.isl-q'); if (q) { q.focus(); q.select(); }");
        });
    };
    let mut open_eq = move || {
        let s = snap.peek();
        eq_local.set(Some((s.eq_enabled, s.eq_bands, s.eq_preset.clone())));
        drop(s);
        go(Mode::Eq);
    };
    let mut choose = move |list: Vec<Track>, i: usize| {
        send(IslandCmd::Play(list, i));
        go(Mode::Card);
    };

    let s = snap.read().clone();
    let m = mode();
    let ph = phase();
    // проценты — как в плеере (100% = обычная громкость), полоса — до предела усиления
    let vol = vol_override().unwrap_or(if s.muted { 0.0 } else { s.volume });
    let vol_pct = (vol * 100.0).round();
    let vol_fill = (vol / s.max_volume.max(0.01) * 100.0).clamp(0.0, 100.0);
    let pct = if s.dur_ms > 0 { (s.pos_ms as f64 / s.dur_ms as f64 * 100.0).min(100.0) } else { 0.0 };
    let acc = s.accent.clone().filter(|a| !a.trim().is_empty()).unwrap_or_else(|| "#ff8a50".into());
    let title = if s.has_track { s.title.clone() } else { "Nebula".to_string() };
    let long_title = title.chars().count() > 30;
    let vol_icon = if vol <= 0.001 {
        "volume_off"
    } else if vol < 0.5 {
        "volume_low"
    } else if vol > 1.001 {
        "volume_boost"
    } else {
        "volume"
    };
    let phase_cls = match ph {
        Phase::Wait => "wait",
        Phase::Arrive => "arrive",
        Phase::Live => "",
        Phase::Leave => "leave",
    };
    let tid = if s.track_id.is_empty() { "none".to_string() } else { s.track_id.clone() };
    let (eq_on, eq_bands, eq_preset) = eq_local().unwrap_or((s.eq_enabled, s.eq_bands, s.eq_preset.clone()));

    // поиск
    let _ = corpus_gen();
    let q = query();
    let qt = q.trim().to_string();
    let source = src();
    let key_now = format!("{source}:{qt}");
    let (local_hits, recent) = {
        let c = CORPUS.lock();
        let recent: Vec<String> = c.as_ref().map(|c| c.recent.clone()).unwrap_or_default();
        let hits: Vec<Track> = if m == Mode::Search && !qt.is_empty() {
            let ql = qt.to_lowercase();
            c.as_ref()
                .map(|c| c.tracks.iter().filter(|t| t.title.to_lowercase().contains(&ql) || t.artist.to_lowercase().contains(&ql)).take(6).cloned().collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        (hits, recent)
    };
    let is_searching = searching().as_deref() == Some(key_now.as_str());
    let found: Option<Result<Vec<Track>, String>> = online().filter(|(k, _)| *k == key_now).map(|(_, r)| r);
    let src_name = if source == "yt" { "YouTube" } else { "SoundCloud" };

    let art = s.art.clone();
    let art_img = move |size: u32| -> Element {
        match art.clone() {
            Some(a) => rsx! { img { src: "{a}", alt: "", draggable: false } },
            None => rsx! { Icon { name: "music", size } },
        }
    };

    rsx! {
        div { class: "isl {m.class()} {phase_cls}",
            class: if s.playing { "playing" },
            style: "--acc: {acc};",
            onmouseenter: enter,
            onmousemove: moved,
            onmouseleave: leave,
            onwheel: wheel,
            div { class: "isl-body",
                // ── компактная ──
                div { class: "lay lay-compact",
                    for id in once(tid.clone()) {
                        div { key: "{id}", class: "isl-art swap", {art_img(14)} }
                    }
                    div { class: if long_title { "isl-title long" } else { "isl-title" },
                        for id in once(tid.clone()) {
                            div { key: "{id}", class: "isl-title-in swap",
                                span { "{title}" }
                                if long_title {
                                    span { "aria-hidden": "true", "{title}" }
                                }
                            }
                        }
                    }
                    div { class: "isl-bars", i {} i {} i {} i {} i {} }
                }
                // ── сменился трек ──
                div { class: "lay lay-peek-t",
                    for id in once(tid.clone()) {
                        div { key: "{id}", class: "isl-art pk swap", {art_img(18)} }
                    }
                    div { class: "pk-meta",
                        div { class: "pk-label", if s.playing { "Сейчас играет" } else { "На паузе" } }
                        for id in once(tid.clone()) {
                            div { key: "{id}", class: "pk-title swap",
                                "{title}"
                                if !s.artist.is_empty() {
                                    span { class: "pk-artist", " · {s.artist}" }
                                }
                            }
                        }
                    }
                    div { class: "isl-bars big", i {} i {} i {} i {} i {} i {} }
                }
                // ── громкость ──
                div { class: "lay lay-peek-v",
                    Icon { name: vol_icon, size: 18 }
                    div { class: "pv-track",
                        div { class: "pv-fill", style: "width: {vol_fill}%;" }
                    }
                    span { class: "pv-num", "{vol_pct}%" }
                }
                // ── карточка ──
                div { class: "lay lay-card",
                    div { class: "c-top rise", style: "--i: 0;",
                        for id in once(tid.clone()) {
                            div { key: "{id}", class: "isl-art big swap", {art_img(24)} }
                        }
                        div { class: "c-meta",
                            for id in once(tid.clone()) {
                                div { key: "{id}", class: "swap",
                                    div { class: "c-name", if s.has_track { "{s.title}" } else { "Ничего не играет" } }
                                    div { class: "c-artist", "{s.artist}" }
                                }
                            }
                            if let Some(l) = s.lyric.clone() {
                                for line in once(l) {
                                    div { key: "{line}", class: "c-lyric swap", "{line}" }
                                }
                            }
                        }
                        div { class: "c-side",
                            button { class: if s.liked { "isl-btn on" } else { "isl-btn" }, title: "В избранное",
                                onclick: move |_| send(IslandCmd::Like),
                                Icon { name: if s.liked { "heart" } else { "heart_outline" }, size: 16 }
                            }
                            button { class: "isl-btn", title: "Поиск песен", onclick: move |_| open_search(),
                                Icon { name: "search", size: 16 }
                            }
                        }
                    }
                    div { class: "c-prog rise", style: "--i: 1;",
                        span { "{fmt(s.pos_ms)}" }
                        div {
                            class: "c-track",
                            title: "Перемотка — щёлкните или тяните",
                            "data-scrub": "seek",
                            "data-dur": "{s.dur_ms}",
                            style: "--pos: {pct}%;",
                            div { class: "c-fill" }
                            div { class: "c-knob" }
                            div { class: "c-tip" }
                        }
                        span { "{fmt(s.dur_ms)}" }
                    }
                    div { class: "c-ctrl rise", style: "--i: 2;",
                        div { class: "c-vol",
                            button { class: "isl-btn sm", title: "Без звука", onclick: move |_| send(IslandCmd::Mute),
                                Icon { name: vol_icon, size: 16 }
                            }
                            div {
                                class: "c-vol-track",
                                title: "Громкость — тяните или крутите колёсико над островком",
                                "data-scrub": "vol",
                                "data-max": "{s.max_volume}",
                                style: "--pos: {vol_fill}%;",
                                div { class: "c-fill" }
                                div { class: "c-knob" }
                                div { class: "c-tip" }
                            }
                        }
                        div { class: "c-mid",
                            button { class: "isl-btn", title: "Предыдущий", onclick: move |_| send(IslandCmd::Prev), Icon { name: "prev", size: 18 } }
                            button { class: if s.playing { "isl-play on" } else { "isl-play" }, title: if s.playing { "Пауза" } else { "Играть" },
                                onclick: move |_| send(IslandCmd::TogglePlay),
                                span { class: "pp pp-play", Icon { name: "play", size: 20 } }
                                span { class: "pp pp-pause", Icon { name: "pause", size: 20 } }
                            }
                            button { class: "isl-btn", title: "Следующий", onclick: move |_| send(IslandCmd::Next), Icon { name: "next", size: 18 } }
                        }
                        div { class: "c-right",
                            button { class: if s.eq_enabled { "isl-btn sm eq-on" } else { "isl-btn sm" }, title: "Эквалайзер", onclick: move |_| open_eq(),
                                Icon { name: "eq", size: 16 }
                            }
                            button { class: "isl-btn sm", title: "Вернуться в Nebula", onclick: move |_| send(IslandCmd::OpenApp),
                                Icon { name: "to_bottom", size: 16 }
                            }
                        }
                    }
                    match s.next.clone() {
                        Some((t, a)) => rsx! {
                            button { class: "c-next rise", style: "--i: 3;", title: "Включить следующий трек",
                                onclick: move |_| send(IslandCmd::Next),
                                span { class: "c-next-l", "Далее" }
                                span { class: "c-next-t", "{t}" }
                                if !a.is_empty() {
                                    span { class: "c-next-a", "— {a}" }
                                }
                                Icon { name: "next", size: 13 }
                            }
                        },
                        None => rsx! {
                            div { class: "c-next empty rise", style: "--i: 3;",
                                span { class: "c-next-l", "Далее" }
                                span { class: "c-next-t", "в очереди больше ничего нет" }
                            }
                        },
                    }
                }
                // ── эквалайзер ──
                div { class: "lay lay-eq",
                    div { class: "e-head",
                        button { class: "isl-btn sm", title: "Назад", onclick: move |_| go(Mode::Card),
                            Icon { name: "back", size: 16 }
                        }
                        div { class: "e-title", Icon { name: "eq", size: 16 } "Эквалайзер" }
                        button {
                            class: if eq_on { "e-switch on" } else { "e-switch" },
                            title: if eq_on { "Выключить эквалайзер" } else { "Включить эквалайзер" },
                            onclick: move |_| {
                                let s = snap.peek();
                                let (on, bands, preset) = eq_local.peek().clone().unwrap_or((s.eq_enabled, s.eq_bands, s.eq_preset.clone()));
                                drop(s);
                                eq_local.set(Some((!on, bands, preset)));
                                send(IslandCmd::EqEnabled(!on));
                            },
                            span {}
                        }
                    }
                    div { class: "e-presets",
                        for (id, name, vals) in EQ_PRESETS.iter() {
                            button {
                                key: "{id}",
                                class: if eq_on && eq_preset == *id { "e-chip on" } else { "e-chip" },
                                onclick: move |_| {
                                    eq_local.set(Some((true, *vals, id.to_string())));
                                    send(IslandCmd::EqPreset(id.to_string()));
                                },
                                "{name}"
                            }
                        }
                    }
                    div { class: if eq_on { "e-bands" } else { "e-bands off" },
                        for i in 0..10usize {
                            div { key: "{i}", class: "e-band", style: "--i: {i};",
                                span { class: "e-db", "{eq_bands[i]:+.0}" }
                                div { class: "e-track", "data-scrub": "eq", "data-axis": "y", "data-i": "{i}",
                                    div { class: "e-zero" }
                                    div {
                                        class: "e-fill",
                                        style: if eq_bands[i] >= 0.0 {
                                            format!("bottom: 50%; height: {}%;", eq_bands[i] / 24.0 * 100.0)
                                        } else {
                                            format!("top: 50%; height: {}%;", -eq_bands[i] / 24.0 * 100.0)
                                        },
                                    }
                                    div { class: "e-knob", style: "bottom: {(eq_bands[i] + 12.0) / 24.0 * 100.0}%;" }
                                }
                                span { class: "e-freq",
                                    if EQ_FREQS[i] >= 1000.0 { "{EQ_FREQS[i] / 1000.0}k" } else { "{EQ_FREQS[i]}" }
                                }
                            }
                        }
                    }
                }
                // ── поиск ──
                div { class: "lay lay-search",
                    div { class: "s-head",
                        button { class: "isl-btn sm", title: "Назад", onclick: move |_| go(Mode::Card),
                            Icon { name: "back", size: 16 }
                        }
                        label { class: "s-box",
                            Icon { name: "search", size: 15 }
                            input {
                                class: "isl-q",
                                r#type: "text",
                                placeholder: "Найти песню…",
                                spellcheck: "false",
                                autocomplete: "off",
                                value: "{q}",
                                oninput: move |e: FormEvent| query.set(e.value()),
                                onkeydown: move |e: KeyboardEvent| match e.key() {
                                    Key::Enter => run_search(),
                                    Key::Escape => go(Mode::Card),
                                    _ => {}
                                },
                            }
                            if !q.is_empty() {
                                button { class: "s-clear", title: "Очистить", onclick: move |_| query.set(String::new()),
                                    Icon { name: "close", size: 12 }
                                }
                            }
                        }
                        div { class: "s-src",
                            button { class: if source == "sc" { "on" }, title: "Искать в SoundCloud",
                                onclick: move |_| {
                                    src.set("sc");
                                    if online.peek().is_some() { run_search(); }
                                },
                                "SC"
                            }
                            button { class: if source == "yt" { "on" }, title: "Искать на YouTube",
                                onclick: move |_| {
                                    src.set("yt");
                                    if online.peek().is_some() { run_search(); }
                                },
                                "YT"
                            }
                        }
                    }
                    div { class: "s-list",
                        if qt.is_empty() {
                            if !recent.is_empty() {
                                div { class: "s-label", "Недавние запросы" }
                                div { class: "s-chips",
                                    for r in recent.iter().take(8).cloned() {
                                        button { key: "{r}", class: "s-chip",
                                            onclick: {
                                                let r = r.clone();
                                                move |_| {
                                                    query.set(r.clone());
                                                    run_search();
                                                }
                                            },
                                            "{r}"
                                        }
                                    }
                                }
                            }
                            div { class: "s-hint",
                                Icon { name: "sparkles", size: 18 }
                                span { "Пока вы печатаете, ищу в избранном, истории и вашей музыке. Enter — поиск в {src_name}." }
                            }
                        } else {
                            if !local_hits.is_empty() {
                                div { class: "s-label", "В библиотеке" }
                                for (i, t) in local_hits.iter().enumerate() {
                                    button { key: "l-{t.id}", class: "s-row", style: "--i: {i};",
                                        onclick: {
                                            let list = local_hits.clone();
                                            move |_| choose(list.clone(), i)
                                        },
                                        div { class: "s-art",
                                            if let Some(a) = t.art(ArtSize::Small) {
                                                img { src: "{a}", alt: "", draggable: false }
                                            } else {
                                                Icon { name: "music", size: 14 }
                                            }
                                        }
                                        div { class: "s-meta",
                                            div { class: "s-t", "{t.title}" }
                                            div { class: "s-a", "{t.artist}" }
                                        }
                                        span { class: "s-d", "{fmt(t.duration_ms)}" }
                                    }
                                }
                            }
                            div { class: "s-label", "{src_name}" }
                            if is_searching {
                                for i in 0..4 {
                                    div { key: "sk-{i}", class: "s-row skel", style: "--i: {i};",
                                        div { class: "s-art" }
                                        div { class: "s-meta", div { class: "s-t" } div { class: "s-a" } }
                                    }
                                }
                            } else {
                                match found {
                                    Some(Ok(list)) if list.is_empty() => rsx! {
                                        div { class: "s-empty", "Ничего не нашлось — попробуйте иначе" }
                                    },
                                    Some(Ok(list)) => {
                                        let list: Vec<Track> = list.into_iter().take(20).collect();
                                        rsx! {
                                            for (i, t) in list.iter().enumerate() {
                                                button { key: "o-{t.id}", class: "s-row", style: "--i: {i.min(10)};",
                                                    onclick: {
                                                        let list = list.clone();
                                                        move |_| choose(list.clone(), i)
                                                    },
                                                    div { class: "s-art",
                                                        if let Some(a) = t.art(ArtSize::Small) {
                                                            img { src: "{a}", alt: "", draggable: false }
                                                        } else {
                                                            Icon { name: "music", size: 14 }
                                                        }
                                                    }
                                                    div { class: "s-meta",
                                                        div { class: "s-t", "{t.title}" }
                                                        div { class: "s-a", "{t.artist}" }
                                                    }
                                                    span { class: "s-d", "{fmt(t.duration_ms)}" }
                                                }
                                            }
                                        }
                                    }
                                    Some(Err(e)) => rsx! {
                                        div { class: "s-empty err", "{e}" }
                                    },
                                    None => rsx! {
                                        button { class: "s-go", onclick: move |_| run_search(),
                                            Icon { name: "search", size: 15 }
                                            span { class: "s-go-t", "Найти «{qt}» в {src_name}" }
                                            kbd { "Enter" }
                                        }
                                    },
                                }
                            }
                        }
                    }
                }
            }
            svg { class: "isl-wave", "aria-hidden": "true",
                path { id: "isl-wave", d: "M 0 0 L 0 2 L 300 2 L 300 0 Z" }
            }
        }
    }
}

const ISLAND_CSS: &str = r#"
:root {
  /* пружинка с лёгким перелётом */
  --spring: linear(0, 0.063 3%, 0.22 7%, 0.46 12%, 0.7 18%, 0.87 24%, 0.97 30%, 1.025 37%, 1.04 43%, 1.035 50%, 1.015 60%, 1 72%, 0.998 85%, 1);
  --morph: 600ms;
}
html, body { margin: 0; height: 100%; background: transparent !important; overflow: hidden; user-select: none;
  font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; color: #f4f4f4; -webkit-font-smoothing: antialiased; }
#main { height: 100%; display: flex; justify-content: center; align-items: flex-start; }
button { border: none; background: none; color: inherit; font: inherit; cursor: pointer; padding: 0; }
.icon { display: block; }

/* ── сама капля: размеры по состояниям ── */
.isl {
  --lvl: 0;
  position: relative; flex: 0 0 auto;
  width: 300px; height: 40px;
  transition: width var(--morph) var(--spring), height var(--morph) var(--spring);
}
.isl.m-peek-t { width: 364px; height: 58px; }
.isl.m-peek-v { width: 320px; height: 46px; }
.isl.m-card { width: 404px; height: 230px; }
.isl.m-search { width: 404px; height: 412px; }
.isl-body {
  position: absolute; inset: 0;
  background: #000;
  border-radius: 0 0 20px 20px;
  overflow: hidden;
  box-shadow: 0 14px 28px -12px rgba(0, 0, 0, .6);
  transition: border-radius var(--morph) var(--spring), box-shadow 300ms ease;
}
.isl.m-peek-t .isl-body { border-radius: 0 0 26px 26px; }
.isl.m-peek-v .isl-body { border-radius: 0 0 23px 23px; }
.isl.m-card .isl-body, .isl.m-search .isl-body { border-radius: 0 0 32px 32px; }
/* под музыку снизу пульсирует свечение цвета обложки */
.isl.playing .isl-body {
  box-shadow: 0 14px 28px -12px rgba(0, 0, 0, .6), 0 8px 26px -12px color-mix(in srgb, var(--acc) calc(var(--lvl) * 100%), transparent);
}
/* вогнутые «уголки» у верхнего края: капля вытекает из края экрана, как вырез */
.isl::before, .isl::after { content: ""; position: absolute; top: 0; width: 14px; height: 14px; z-index: 1; pointer-events: none; transition: opacity 200ms; }
.isl::before { left: -14px; background: radial-gradient(circle at 0 100%, transparent 13.5px, #000 14px); }
.isl::after { right: -14px; background: radial-gradient(circle at 100% 100%, transparent 13.5px, #000 14px); }
.isl-wave { position: absolute; left: 16px; width: calc(100% - 32px); top: calc(100% - 1px); height: 12px; overflow: visible; pointer-events: none;
  transition: left var(--morph) var(--spring), width var(--morph) var(--spring), opacity 200ms; }
.isl-wave path { fill: #000; }
.isl:not(.m-compact) .isl-wave { left: 28px; width: calc(100% - 56px); }

/* ── появление: капля падает сверху, растекается и чуть пружинит ── */
.isl.wait { width: 34px; height: 0; transition: none; }
.isl.wait .lay, .isl.wait .isl-wave, .isl.wait::before, .isl.wait::after { opacity: 0; }
.isl.arrive { animation: isl-arrive 780ms both; }
.isl.arrive .isl-body { animation: isl-arrive-r 780ms both; }
.isl.arrive::before, .isl.arrive::after, .isl.arrive .isl-wave { animation: fade-in 220ms ease 380ms both; }
.isl.arrive .lay-compact { animation: fade-in 320ms ease 400ms both; }
@keyframes isl-arrive {
  0% { width: 34px; height: 0; animation-timing-function: cubic-bezier(.3, .8, .4, 1); }
  26% { width: 38px; height: 34px; animation-timing-function: cubic-bezier(.6, 0, .3, 1); }
  54% { width: 312px; height: 35px; animation-timing-function: cubic-bezier(.3, .6, .5, 1); }
  71% { width: 293px; height: 43px; }
  86% { width: 303px; height: 39px; }
  100% { width: 300px; height: 40px; }
}
@keyframes isl-arrive-r { 0%, 26% { border-radius: 0 0 50% 50%; } 54%, 100% { border-radius: 0 0 20px 20px; } }
@keyframes fade-in { from { opacity: 0; } }
/* уход: втягивается обратно в край */
.isl.leave { width: 34px !important; height: 0 !important; transition: width 300ms cubic-bezier(.6, 0, .8, .2), height 300ms cubic-bezier(.6, 0, .8, .2) !important; }
.isl.leave .lay, .isl.leave .isl-wave, .isl.leave::before, .isl.leave::after { opacity: 0 !important; transition: opacity 120ms !important; }

/* ── слои состояний: перетекают с размытием, а не щёлкают ── */
.lay {
  position: absolute; top: 0; left: 50%; translate: -50% 0; box-sizing: border-box;
  opacity: 0; scale: .94; filter: blur(6px); pointer-events: none;
  transition: opacity 170ms ease, scale 420ms var(--spring), filter 220ms ease;
}
.isl.m-compact .lay-compact, .isl.m-peek-t .lay-peek-t, .isl.m-peek-v .lay-peek-v, .isl.m-card .lay-card, .isl.m-search .lay-search {
  opacity: 1; scale: 1; filter: none; pointer-events: auto;
  transition: opacity 260ms ease 110ms, scale 560ms var(--spring) 60ms, filter 300ms ease 90ms;
}
@keyframes rise { from { opacity: 0; translate: 0 10px; filter: blur(4px); } }
@keyframes swap { from { opacity: 0; translate: 0 7px; filter: blur(5px); scale: .97; } }
.swap { animation: swap 520ms var(--spring) both; }
.isl.m-card .lay-card .rise { animation: rise 560ms var(--spring) both; animation-delay: calc(120ms + var(--i, 0) * 45ms); }

/* ── компактная ── */
.lay-compact { width: 300px; height: 40px; display: flex; align-items: center; gap: 10px; padding: 0 15px 0 7px; }
.isl-art { width: 26px; height: 26px; flex: 0 0 auto; border-radius: 8px; overflow: hidden; display: grid; place-items: center; background: #1c1c1c; color: #888; }
.isl-art img { width: 100%; height: 100%; object-fit: cover; }
.lay-compact .isl-art { scale: calc(1 + var(--lvl) * .1); transition: scale 90ms linear; }
.isl:not(.playing) .lay-compact .isl-art { filter: saturate(.4) brightness(.8); }
.isl-title { flex: 1; min-width: 0; overflow: hidden; font-size: 13px; font-weight: 600; white-space: nowrap; }
.isl-title-in { overflow: hidden; text-overflow: ellipsis; }
/* длинное название едет бегущей строкой */
.isl-title.long { mask-image: linear-gradient(90deg, transparent, #000 8px, #000 calc(100% - 14px), transparent); }
.isl-title.long .isl-title-in { overflow: visible; display: inline-flex; }
.isl-title.long .isl-title-in span { padding-right: 36px; animation: marquee 14s linear 1.6s infinite; }
@keyframes marquee { to { translate: -100% 0; } }
.isl-bars { display: flex; align-items: center; gap: 2px; height: 18px; flex: 0 0 auto; }
.isl-bars i { width: 3px; height: calc(18% + var(--h, 0) * 82%); border-radius: 2px;
  background: linear-gradient(var(--acc), color-mix(in srgb, var(--acc) 55%, #ff3d6e)); transition: height 90ms linear, opacity 300ms; }
.isl-bars.big { height: 24px; gap: 3px; }
.isl:not(.playing) .isl-bars i { height: 18%; opacity: .45; }

/* ── сменился трек ── */
.lay-peek-t { width: 364px; height: 58px; display: flex; align-items: center; gap: 12px; padding: 0 18px 0 9px; }
.isl-art.pk { width: 40px; height: 40px; border-radius: 12px; }
.pk-meta { flex: 1; min-width: 0; }
.pk-label { font-size: 10.5px; font-weight: 700; letter-spacing: .04em; text-transform: uppercase; color: var(--acc); }
.pk-title { margin-top: 1px; font-size: 13.5px; font-weight: 650; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.pk-artist { color: #9a9a9a; font-weight: 500; }

/* ── громкость ── */
.lay-peek-v { width: 320px; height: 46px; display: flex; align-items: center; gap: 12px; padding: 0 18px; color: #e9e9e9; }
.pv-track { flex: 1; height: 6px; border-radius: 6px; background: rgba(255, 255, 255, .14); overflow: hidden; }
.pv-fill { height: 100%; border-radius: inherit; background: linear-gradient(90deg, #fff, var(--acc)); transition: width 160ms var(--spring); }
.pv-num { width: 38px; text-align: right; font-size: 12px; font-weight: 650; font-variant-numeric: tabular-nums; color: #bdbdbd; }

/* ── карточка ── */
.lay-card { width: 404px; height: 230px; padding: 15px 18px 12px; display: flex; flex-direction: column; }
.c-top { display: flex; gap: 12px; align-items: flex-start; min-height: 64px; }
.isl-art.big { width: 64px; height: 64px; border-radius: 15px; box-shadow: 0 8px 20px -8px color-mix(in srgb, var(--acc) 70%, transparent); }
.c-meta { flex: 1; min-width: 0; padding-top: 1px; }
.c-name { font-size: 15px; font-weight: 700; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-artist { font-size: 12px; color: #9a9a9a; margin-top: 2px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.c-lyric { margin-top: 7px; font-size: 12.5px; font-weight: 600; color: #ececec; line-height: 1.3; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.c-side { display: flex; flex-direction: column; gap: 2px; margin: -4px -6px 0 0; }
.isl-btn { width: 34px; height: 34px; border-radius: 50%; display: grid; place-items: center; color: #bdbdbd; transition: background 160ms, color 160ms, scale 300ms var(--spring); }
.isl-btn.sm { width: 30px; height: 30px; }
.isl-btn:hover { background: rgba(255, 255, 255, .1); color: #fff; }
.isl-btn:active { scale: .88; }
.isl-btn.on { color: #ff6b7d; }
.isl-btn.on .icon { animation: pop 520ms var(--spring); }
@keyframes pop { 0% { scale: .4; } 45% { scale: 1.3; } 100% { scale: 1; } }
.c-prog { display: flex; align-items: center; gap: 10px; margin-top: auto; font-size: 11px; color: #8d8d8d; font-variant-numeric: tabular-nums; }
.c-track, .c-vol-track { position: relative; height: 4px; border-radius: 4px; background: rgba(255, 255, 255, .16); cursor: pointer; touch-action: none;
  transition: height 200ms var(--spring); }
.c-track { flex: 1; }
.c-track:hover, .c-track.drag, .c-vol-track:hover, .c-vol-track.drag { height: 6px; }
.c-track::before, .c-vol-track::before { content: ""; position: absolute; inset: -9px 0; }
.c-fill { position: absolute; left: 0; top: 0; bottom: 0; width: var(--pos, 0%); border-radius: inherit; background: linear-gradient(90deg, #fff 60%, var(--acc));
  transition: width 160ms linear; pointer-events: none; }
/* ползунок и подсказка с временем появляются под мышью; пока тянут — всё идёт за курсором */
.c-knob { position: absolute; top: 50%; left: var(--pos, 0%); width: 12px; height: 12px; margin: -6px 0 0 -6px; border-radius: 50%; background: #fff; pointer-events: none;
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--acc) 35%, transparent); scale: 0; transition: scale 260ms var(--spring), left 160ms linear; }
.c-track:hover .c-knob, .c-vol-track:hover .c-knob, .drag .c-knob { scale: 1; }
.drag .c-fill { width: calc(var(--drag) * 100%); transition: none; }
.drag .c-knob { left: calc(var(--drag) * 100%); transition: scale 260ms var(--spring); }
.c-tip { position: absolute; bottom: 12px; left: calc(var(--drag, 0) * 100%); translate: -50% 0; padding: 2px 6px; border-radius: 6px; background: #fff; color: #000;
  font-size: 10.5px; font-weight: 700; font-variant-numeric: tabular-nums; white-space: nowrap; pointer-events: none; opacity: 0; scale: .8; transition: opacity 140ms, scale 220ms var(--spring); }
.drag .c-tip:not(:empty) { opacity: 1; scale: 1; }
.isl-btn.eq-on { color: var(--acc); }
.c-ctrl { display: grid; grid-template-columns: 1fr auto 1fr; align-items: center; margin-top: 8px; }
.c-vol { display: flex; align-items: center; gap: 4px; margin-left: -6px; }
.c-vol-track { width: 62px; }
.c-mid { display: flex; align-items: center; gap: 12px; }
.c-right { display: flex; justify-content: flex-end; gap: 2px; margin-right: -6px; }
.isl-play { position: relative; width: 44px; height: 44px; border-radius: 50%; background: #fff; color: #000; transition: scale 300ms var(--spring), box-shadow 300ms; }
.isl-play:hover { scale: 1.07; box-shadow: 0 0 0 6px rgba(255, 255, 255, .08); }
.isl-play:active { scale: .9; }
/* значки play/pause перетекают друг в друга с поворотом */
.pp { position: absolute; inset: 0; display: grid; place-items: center; transition: opacity 200ms ease, rotate 420ms var(--spring), scale 420ms var(--spring); }
.isl-play .pp-pause, .isl-play.on .pp-play { opacity: 0; rotate: -90deg; scale: .4; }
.isl-play.on .pp-pause { opacity: 1; rotate: 0deg; scale: 1; }
.c-next { display: flex; align-items: center; gap: 8px; margin-top: 9px; height: 28px; padding: 0 10px; border-radius: 10px; background: rgba(255, 255, 255, .055);
  font-size: 12px; color: #d6d6d6; text-align: left; min-width: 0; transition: background 160ms; }
.c-next:not(.empty):hover { background: rgba(255, 255, 255, .1); }
.c-next .icon { margin-left: auto; flex: 0 0 auto; color: #8d8d8d; }
.c-next-l { flex: 0 0 auto; font-size: 10.5px; font-weight: 700; text-transform: uppercase; letter-spacing: .04em; color: var(--acc); }
.c-next-t { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 600; }
.c-next-a { flex: 0 1 auto; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: #8d8d8d; }
.c-next.empty .c-next-t { color: #777; font-weight: 500; }

/* ── эквалайзер ── */
.isl.m-eq { width: 404px; height: 300px; }
.isl.m-eq .isl-body { border-radius: 0 0 32px 32px; }
.isl.m-eq .lay-eq { opacity: 1; scale: 1; filter: none; pointer-events: auto; transition: opacity 260ms ease 110ms, scale 560ms var(--spring) 60ms, filter 300ms ease 90ms; }
.lay-eq { width: 404px; height: 300px; padding: 14px 16px 14px; display: flex; flex-direction: column; gap: 10px; }
.e-head { display: flex; align-items: center; gap: 8px; }
.e-title { flex: 1; display: flex; align-items: center; gap: 8px; font-size: 14px; font-weight: 700; }
.e-title .icon { color: var(--acc); }
.e-switch { width: 40px; height: 24px; border-radius: 12px; background: #2a2a2a; position: relative; transition: background 200ms; }
.e-switch span { position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: #9a9a9a; transition: translate 320ms var(--spring), background 200ms; }
.e-switch.on { background: var(--acc); }
.e-switch.on span { translate: 16px 0; background: #fff; }
.e-presets { display: flex; gap: 6px; overflow-x: auto; scrollbar-width: none; mask-image: linear-gradient(90deg, #000 85%, transparent); padding-right: 24px; flex: 0 0 auto; }
.e-presets::-webkit-scrollbar { display: none; }
.e-chip { flex: 0 0 auto; height: 28px; padding: 0 11px; border-radius: 14px; background: rgba(255, 255, 255, .07); font-size: 12px; color: #d6d6d6; transition: background 160ms, color 160ms, scale 260ms var(--spring); }
.e-chip:hover { background: rgba(255, 255, 255, .13); }
.e-chip:active { scale: .94; }
.e-chip.on { background: #fff; color: #000; font-weight: 650; }
.e-bands { flex: 1; min-height: 0; display: flex; justify-content: space-between; padding: 0 2px; transition: opacity 250ms; }
.e-bands.off { opacity: .4; }
.e-band { width: 30px; display: flex; flex-direction: column; align-items: center; gap: 6px; }
.e-db { font-size: 10.5px; font-weight: 650; color: #bdbdbd; font-variant-numeric: tabular-nums; }
.e-track { position: relative; flex: 1; width: 6px; border-radius: 6px; background: rgba(255, 255, 255, .1); cursor: ns-resize; touch-action: none; }
.e-track::before { content: ""; position: absolute; inset: 0 -12px; }
.e-zero { position: absolute; left: -4px; right: -4px; top: 50%; height: 1px; background: rgba(255, 255, 255, .22); pointer-events: none; }
.e-fill { position: absolute; left: 0; right: 0; border-radius: 6px; background: linear-gradient(var(--acc), color-mix(in srgb, var(--acc) 55%, #ff3d6e)); pointer-events: none;
  transition: height 120ms var(--spring), top 120ms, bottom 120ms; }
.e-knob { position: absolute; left: 50%; width: 14px; height: 14px; margin: 0 0 -7px -7px; border-radius: 50%; background: #fff; pointer-events: none;
  box-shadow: 0 2px 6px rgba(0, 0, 0, .5); transition: bottom 120ms var(--spring), scale 220ms var(--spring); }
.e-track:hover .e-knob { scale: 1.15; }
.e-freq { font-size: 10px; color: #6f6f6f; font-variant-numeric: tabular-nums; }
.isl.m-eq .e-band { animation: rise 480ms var(--spring) both; animation-delay: calc(100ms + var(--i, 0) * 20ms); }

/* ── поиск ── */
.lay-search { width: 404px; height: 412px; padding: 14px 14px 12px; display: flex; flex-direction: column; }
.s-head { display: flex; align-items: center; gap: 8px; }
.s-box { flex: 1; min-width: 0; height: 38px; display: flex; align-items: center; gap: 8px; padding: 0 10px 0 12px; border-radius: 13px; background: #151515; color: #8d8d8d;
  box-shadow: inset 0 0 0 1px rgba(255, 255, 255, .06); transition: box-shadow 200ms, background 200ms; }
.s-box:focus-within { background: #1a1a1a; box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--acc) 60%, transparent); }
.isl-q { flex: 1; min-width: 0; border: none; outline: none; background: transparent; color: #f4f4f4; font: inherit; font-size: 13.5px; user-select: text; }
.isl-q::placeholder { color: #6f6f6f; }
.s-clear { width: 20px; height: 20px; border-radius: 50%; display: grid; place-items: center; background: rgba(255, 255, 255, .1); color: #ccc; }
.s-src { display: flex; padding: 3px; gap: 2px; border-radius: 11px; background: #151515; }
.s-src button { height: 30px; padding: 0 9px; border-radius: 8px; font-size: 11.5px; font-weight: 700; color: #8d8d8d; transition: background 160ms, color 160ms; }
.s-src button.on { background: #fff; color: #000; }
.s-list { flex: 1; min-height: 0; margin-top: 10px; overflow-y: auto; overflow-x: hidden; padding-right: 2px; }
.s-list::-webkit-scrollbar { width: 6px; }
.s-list::-webkit-scrollbar-thumb { background: rgba(255, 255, 255, .14); border-radius: 6px; }
.s-label { margin: 8px 6px 4px; font-size: 10.5px; font-weight: 700; letter-spacing: .05em; text-transform: uppercase; color: #6f6f6f; }
.s-label:first-child { margin-top: 2px; }
.s-row { width: 100%; display: flex; align-items: center; gap: 10px; padding: 5px 8px 5px 6px; border-radius: 11px; text-align: left; transition: background 140ms;
  animation: rise 420ms var(--spring) both; animation-delay: calc(var(--i, 0) * 26ms); }
.s-row:hover { background: rgba(255, 255, 255, .07); }
.s-row:active { background: rgba(255, 255, 255, .11); }
.s-art { width: 36px; height: 36px; flex: 0 0 auto; border-radius: 9px; overflow: hidden; display: grid; place-items: center; background: #1a1a1a; color: #777; }
.s-art img { width: 100%; height: 100%; object-fit: cover; }
.s-meta { flex: 1; min-width: 0; }
.s-t { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.s-a { margin-top: 1px; font-size: 11.5px; color: #8d8d8d; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.s-d { flex: 0 0 auto; font-size: 11px; color: #6f6f6f; font-variant-numeric: tabular-nums; }
.s-row.skel { pointer-events: none; }
.s-row.skel .s-art, .s-row.skel .s-t, .s-row.skel .s-a { background: linear-gradient(90deg, #161616 25%, #232323 50%, #161616 75%); background-size: 200% 100%; animation: shimmer 1.2s linear infinite; border-radius: 6px; }
.s-row.skel .s-art { border-radius: 9px; }
.s-row.skel .s-t { height: 11px; width: 70%; }
.s-row.skel .s-a { height: 9px; width: 40%; margin-top: 6px; }
@keyframes shimmer { to { background-position: -200% 0; } }
.s-go { width: 100%; display: flex; align-items: center; gap: 10px; padding: 10px 10px 10px 12px; border-radius: 12px; background: rgba(255, 255, 255, .05); color: #d6d6d6; font-size: 12.5px; text-align: left; transition: background 140ms; }
.s-go:hover { background: rgba(255, 255, 255, .09); }
.s-go-t { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
kbd { font: inherit; font-size: 10.5px; font-weight: 700; color: #9a9a9a; padding: 2px 6px; border-radius: 6px; background: rgba(255, 255, 255, .08); }
.s-chips { display: flex; flex-wrap: wrap; gap: 6px; padding: 0 4px; }
.s-chip { height: 28px; padding: 0 11px; border-radius: 14px; background: rgba(255, 255, 255, .07); font-size: 12px; color: #d6d6d6; transition: background 140ms; animation: rise 380ms var(--spring) both; }
.s-chip:hover { background: rgba(255, 255, 255, .13); }
.s-hint { display: flex; gap: 10px; align-items: flex-start; margin: 14px 6px 0; padding: 12px; border-radius: 12px; background: rgba(255, 255, 255, .04); color: #8d8d8d; font-size: 12px; line-height: 1.4; }
.s-hint .icon { flex: 0 0 auto; color: var(--acc); }
.s-empty { padding: 14px 8px; font-size: 12.5px; color: #8d8d8d; }
.s-empty.err { color: #ff8a8a; }
"#;
