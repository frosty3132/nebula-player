//! Тексты песен: боковая панель, полноэкранный режим и ручной поиск в LRCLIB.
//! Активная строка вычисляется мемо-сигналом — список перерисовывается только
//! при смене строки, а «караоке»-заливка идёт CSS-анимацией на GPU.

use super::actions::*;
use super::icons::Icon;
use super::player_bar::{Controls, Cover, Progress, Volume};
use super::state::*;
use super::widgets::{IconBtn, Spinner};
use crate::lyrics::{guess_artist_title, Lyrics};
use crate::model::{fmt_time, ArtSize};
use crate::services::svc;
use crate::settings::LyricsAlign;
use dioxus::desktop::window;
use dioxus::prelude::*;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Прокрутка текста к активной строке «волной», как в Apple Music: список
/// сразу встаёт на место, а каждая строка догоняет его со своей задержкой
/// (FLIP через Web Animations, свойство `translate` — не мешает масштабу
/// активной строки). Без анимаций или при большом прыжке — сразу на место.
const LYRICS_SCROLL_JS: &str = r#"(() => {
  const el = document.getElementById("__ID__");
  if (!el) return;
  const sc = el.closest(".ly-body");
  if (!sc) { el.scrollIntoView({ block: "center" }); return; }
  const r = el.getBoundingClientRect(), b = sc.getBoundingClientRect();
  const max = sc.scrollHeight - sc.clientHeight;
  const target = Math.max(0, Math.min(max, sc.scrollTop + (r.top - b.top) - (sc.clientHeight - r.height) / 2));
  const delta = target - sc.scrollTop;
  if (Math.abs(delta) < 1) return;
  sc.scrollTo({ top: target, behavior: "instant" });
  if (!__SMOOTH__ || Math.abs(delta) > sc.clientHeight * 1.5) return;
  const k = parseFloat(getComputedStyle(sc).getPropertyValue("--anim")) || 1;
  const lines = Array.from(sc.querySelectorAll(".ly-line"));
  const ai = lines.indexOf(el);
  lines.forEach((ln, i) => {
    const d = i - ai;
    if (d < -6 || d > 10) return; // за краями всё равно не видно
    // строки выше активной трогаются первыми, ниже — с нарастающей задержкой
    const delay = Math.max(0, d + 2) * 38 * k;
    ln.getAnimations().forEach((a) => { if (a.id === "ly-wave") a.cancel(); });
    const anim = ln.animate([{ translate: `0 ${delta}px` }, { translate: "0 0" }], { duration: 640 * k, delay, easing: "cubic-bezier(.22, 1, .36, 1)", fill: "backwards" });
    anim.id = "ly-wave";
  });
})()"#;

#[component]
fn LyricsBody(prefix: String, #[props(default)] big: bool) -> Element {
    let st = app_state();
    let (font, align, blur, karaoke, autoscroll, global_off, plain_ok) = {
        let s = st.settings.read();
        (s.lyrics_font, s.lyrics_align, s.lyrics_blur, s.lyrics_karaoke, s.lyrics_autoscroll, s.lyrics_offset_ms, s.lyrics_plain_fallback)
    };
    let active = use_memo(move || {
        let off = *st.lyrics_offset.read() as i64 + st.settings.read().lyrics_offset_ms as i64;
        match &*st.lyrics.read() {
            LyricsState::Found(l) => l.active_index(*st.position.read() as i64 + off),
            _ => None,
        }
    });
    let last_wheel: Rc<Cell<Option<Instant>>> = use_hook(|| Rc::new(Cell::new(None)));
    let lw = last_wheel.clone();
    let px = prefix.clone();
    use_effect(move || {
        let a = active();
        if !st.settings.peek().lyrics_autoscroll {
            return;
        }
        if lw.get().is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
            return;
        }
        let smooth = st.settings.peek().animations && !st.settings.peek().low_power;
        let id = match a {
            Some(i) => format!("{px}-{i}"),
            None => format!("{px}-top"),
        };
        document::eval(&LYRICS_SCROLL_JS.replace("__ID__", &id).replace("__SMOOTH__", if smooth { "true" } else { "false" }));
    });

    let state = st.lyrics.read().clone();
    let align_class = match align {
        LyricsAlign::Left => "al-left",
        LyricsAlign::Center => "al-center",
        LyricsAlign::Right => "al-right",
    };
    let size = if big { font * 1.6 } else { font };
    let style = format!("--lyrics-fs: {size}px;");
    let body_class = format!("ly-body {align_class}{}", if blur { " blur" } else { "" });

    let content = match state {
        LyricsState::Idle => rsx! {
            div { class: "ly-empty", Icon { name: "lyrics", size: 30 } p { "Включите тексты в настройках или выберите трек" } }
        },
        LyricsState::Loading => rsx! {
            div { class: "ly-loading",
                for i in 0..7 {
                    div { key: "{i}", class: "sk sk-line", style: "width: {55 + (i * 37) % 40}%;" }
                }
            }
        },
        LyricsState::NotFound | LyricsState::Failed(_) => {
            let msg = if let LyricsState::Failed(e) = &state { e.clone() } else { "Для этого трека в LRCLIB нет текста".to_string() };
            rsx! {
                div { class: "ly-empty",
                    Icon { name: "mic", size: 30 }
                    p { "{msg}" }
                    button { class: "btn small", onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); }, "Найти вручную" }
                }
            }
        }
        LyricsState::Found(l) => {
            if l.instrumental && !l.is_synced() {
                rsx! { div { class: "ly-empty", span { class: "ly-note", "♪" } p { "Инструментальная композиция" } } }
            } else if l.is_synced() {
                let a = active();
                let off = *st.lyrics_offset.read() as i64 + global_off as i64;
                let pos = *st.position.peek() as i64 + off;
                let lines = l.synced.clone();
                rsx! {
                    div { id: "{prefix}-top", class: "ly-spacer" }
                    for (i , line) in lines.iter().enumerate() {
                        {
                            let is_active = a == Some(i);
                            let past = a.is_some_and(|x| i < x);
                            let dist = a.map(|x| (i as i64 - x as i64).unsigned_abs()).unwrap_or(4).min(6);
                            let next_t = lines.get(i + 1).map(|n| n.time_ms).unwrap_or(line.time_ms + 4000);
                            let dur = next_t.saturating_sub(line.time_ms).clamp(300, 12_000);
                            let elapsed = (pos - line.time_ms as i64).clamp(0, dur as i64);
                            let seek_to = (line.time_ms as i64 - off).max(0) as u64;
                            let text = if line.text.trim().is_empty() { "♪".to_string() } else { line.text.clone() };
                            let class = if is_active { "ly-line active" } else if past { "ly-line past" } else { "ly-line" };
                            rsx! {
                                div {
                                    key: "{i}",
                                    id: "{prefix}-{i}",
                                    class: "{class}",
                                    "data-el": if is_active { "lyrics_active" } else { "lyrics_line" },
                                    style: "--d: {dist};",
                                    onclick: move |_| seek(st, seek_to),
                                    if is_active && karaoke {
                                        span { class: "ly-k", "data-text": "{text}", style: "animation-duration: {dur}ms; animation-delay: -{elapsed}ms;", "{text}" }
                                    } else {
                                        "{text}"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "ly-spacer" }
                }
            } else if let (true, Some(plain)) = (plain_ok, l.plain.clone()) {
                rsx! {
                    div { class: "ly-plain-note", "Текст без синхронизации" }
                    div { class: "ly-plain", "{plain}" }
                }
            } else {
                rsx! { div { class: "ly-empty", p { "Есть только несинхронизированный текст (включите показ в настройках)" } } }
            }
        }
    };

    rsx! {
        div {
            class: "{body_class}",
            style: "{style}",
            onwheel: move |_| last_wheel.set(Some(Instant::now())),
            {content}
        }
        if !autoscroll {
            span { class: "hidden" }
        }
    }
}

#[component]
fn OffsetControls() -> Element {
    let st = app_state();
    let off = *st.lyrics_offset.read();
    let label = if off == 0 { "0.0 с".to_string() } else { format!("{:+.1} с", off as f64 / 1000.0) };
    rsx! {
        div { class: "ly-offset", title: "Сдвиг текста для этого трека",
            button { class: "mini", onclick: move |_| set_lyrics_offset(st, -250), "−" }
            span { class: "ly-off-val", ondoubleclick: move |_| set_lyrics_offset(st, -off), "{label}" }
            button { class: "mini", onclick: move |_| set_lyrics_offset(st, 250), "+" }
        }
    }
}

#[component]
pub fn LyricsPanel() -> Element {
    let st = app_state();
    let found = matches!(&*st.lyrics.read(), LyricsState::Found(_));
    let source = match &*st.lyrics.read() {
        LyricsState::Found(l) if l.is_synced() => "LRCLIB · синхронно",
        LyricsState::Found(_) => "LRCLIB",
        _ => "",
    };
    rsx! {
        aside { class: "lyrics glass", "data-el": "lyrics",
            div { class: "ly-head",
                div { class: "ly-title",
                    Icon { name: "lyrics", size: 18 }
                    span { "Текст" }
                }
                div { class: "flex1" }
                IconBtn { icon: "search", size: 17, title: "Найти текст вручную", onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); } }
                IconBtn { icon: "fullscreen", size: 17, title: "На весь экран (F)", onclick: move |_| { let mut f = st.fullscreen; f.set(true); } }
                IconBtn { icon: "close", size: 17, title: "Скрыть (L)", onclick: move |_| { let mut s = st.settings; s.write().lyrics_open = false; } }
            }
            if found {
                div { class: "ly-sub",
                    span { class: "badge soft", "{source}" }
                    div { class: "flex1" }
                    OffsetControls {}
                }
            }
            LyricsBody { prefix: "lp" }
        }
    }
}

#[component]
pub fn FullscreenLyrics() -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let art = now.as_ref().and_then(|t| t.art(ArtSize::Large));
    let order = st.theme.read().layout.controls.clone();
    let lv = if st.settings.read().beat_glow { (*st.level.read() * 3.2).min(1.0) } else { 0.0 };
    let mut win_fs = use_signal(|| false);
    rsx! {
        div { class: "fs-lyrics", "data-el": "lyrics_full",
            if let Some(a) = art.clone() {
                div { class: "fs-bg", style: "background-image: url('{a}');" }
            }
            div { class: "fs-veil" }
            div { class: "fs-top",
                div { class: "flex1" }
                OffsetControls {}
                IconBtn { icon: "search", title: "Найти текст вручную", onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::LyricsSearch)); } }
                IconBtn {
                    icon: if *win_fs.read() { "fullscreen_exit" } else { "fullscreen" },
                    title: "Полноэкранное окно (F11)",
                    onclick: move |_| {
                        let v = !*win_fs.peek();
                        window().set_fullscreen(v);
                        win_fs.set(v);
                    },
                }
                IconBtn { icon: "close", title: "Закрыть (Esc)",
                    onclick: move |_| {
                        if *win_fs.peek() {
                            window().set_fullscreen(false);
                        }
                        let mut f = st.fullscreen;
                        f.set(false);
                    } }
            }
            div { class: "fs-content",
                div { class: "fs-left",
                    div { class: "fs-cover", style: "--beat: {lv:.2};",
                        span { class: "fs-glow" }
                        Cover { src: art }
                    }
                    if let Some(t) = now {
                        div { class: "fs-title", "{t.title}" }
                        div { class: "fs-artist", "{t.artist}" }
                    }
                    div { class: "fs-controls",
                        Progress {}
                        Controls { order }
                        div { class: "fs-volume", Volume {} }
                    }
                }
                div { class: "fs-right", LyricsBody { prefix: "lf", big: true } }
            }
        }
    }
}

/// Содержимое окна ручного поиска текста.
#[component]
pub fn LyricsSearch() -> Element {
    let st = app_state();
    let now = st.now.read().clone();
    let initial = now.as_ref().map(|t| {
        let (a, ti) = guess_artist_title(t);
        format!("{a} {ti}").trim().to_string()
    });
    let mut q = use_signal(|| initial.clone().unwrap_or_default());
    let mut results = use_signal(Vec::<Lyrics>::new);
    let mut busy = use_signal(|| false);
    let mut err = use_signal(|| None::<String>);
    let run = move || {
        let query = q.peek().clone();
        if query.trim().is_empty() {
            return;
        }
        busy.set(true);
        err.set(None);
        spawn(async move {
            match bg(async move { svc().lyrics.search(&query).await }).await {
                Some(Ok(list)) => results.set(list),
                Some(Err(e)) => err.set(Some(e)),
                None => {}
            }
            busy.set(false);
        });
    };
    let mut run2 = run;
    use_hook(move || run2());
    let Some(track) = now else {
        return rsx! { p { "Сначала включите трек." } };
    };
    let tid = track.id.clone();
    let tid2 = track.id.clone();
    let dur = track.duration_ms;
    rsx! {
        div { class: "lyr-search",
            div { class: "filter-bar",
                Icon { name: "search", size: 18 }
                input {
                    class: "input bare",
                    value: "{q}",
                    placeholder: "Исполнитель и название",
                    oninput: move |e: FormEvent| q.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Enter {
                            let mut r = run;
                            r();
                        }
                    },
                }
                button { class: "btn small", onclick: move |_| { let mut r = run; r(); }, "Искать" }
            }
            if *busy.read() {
                div { class: "center", Spinner {} }
            }
            if let Some(e) = err.read().clone() {
                div { class: "error-box", "{e}" }
            }
            div { class: "lyr-results",
                for (i , l) in results.read().iter().cloned().enumerate() {
                    {
                        let diff = (l.duration_s * 1000.0 - dur as f64).abs() / 1000.0;
                        let tid = tid.clone();
                        let l2 = l.clone();
                        rsx! {
                            button {
                                key: "{i}",
                                class: "lyr-item",
                                onclick: move |_| {
                                    svc().lyrics.set_manual(&tid, Some(l2.clone()));
                                    let mut ly = st.lyrics;
                                    ly.set(LyricsState::Found(l2.clone()));
                                    let mut m = st.modal;
                                    m.set(None);
                                    toast(st, ToastKind::Success, "Текст выбран и сохранён");
                                },
                                div { class: "lyr-main",
                                    div { class: "lyr-name ellipsis", "{l.track_name}" }
                                    div { class: "lyr-artist ellipsis", "{l.artist_name}" }
                                }
                                span { class: if diff < 3.0 { "badge ok" } else { "badge soft" }, "{fmt_time((l.duration_s * 1000.0) as u64)}" }
                                if l.is_synced() {
                                    span { class: "badge", "синхр." }
                                } else if l.instrumental {
                                    span { class: "badge soft", "инстр." }
                                } else {
                                    span { class: "badge soft", "текст" }
                                }
                            }
                        }
                    }
                }
            }
            div { class: "modal-actions",
                button { class: "btn ghost",
                    onclick: move |_| {
                        svc().lyrics.set_manual(&tid2, None);
                        let mut ly = st.lyrics;
                        ly.set(LyricsState::NotFound);
                        let mut m = st.modal;
                        m.set(None);
                    },
                    "У трека нет текста"
                }
            }
        }
    }
}
