//! Действия интерфейса: навигация, очередь, воспроизведение, избранное,
//! тексты, предзагрузка следующего трека, уведомления.
//! Сетевые операции выполняются на tokio-рантайме, UI только ждёт результат.

use super::state::*;
use crate::audio::{MediaSpec, PlaybackState};
use crate::library::SavedSession;
use crate::model::{ArtSize, Track};
use crate::services::svc;
use crate::settings::{RepeatMode, StreamQuality};
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static TOAST_ID: AtomicU64 = AtomicU64::new(1);

/// Выполнить future на фоновом рантайме и дождаться результата из UI.
pub async fn bg<T: Send + 'static>(fut: impl Future<Output = T> + Send + 'static) -> Option<T> {
    svc().rt.spawn(fut).await.ok()
}

pub fn toast(state: AppState, kind: ToastKind, text: impl Into<String>) {
    let id = TOAST_ID.fetch_add(1, Ordering::Relaxed);
    let mut toasts = state.toasts;
    {
        let mut w = toasts.write();
        w.push(Toast { id, kind, text: text.into() });
        while w.len() > 4 {
            w.remove(0);
        }
    }
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_millis(4200)).await;
        toasts.write().retain(|t| t.id != id);
    });
}

pub fn navigate(state: AppState, route: Route) {
    let mut st = state;
    let cur = st.route.peek().clone();
    if cur == route {
        return;
    }
    let same_page = cur.key() == route.key();
    {
        let mut back = st.back.write();
        back.push(cur);
        if back.len() > 60 {
            back.remove(0);
        }
    }
    st.forward.write().clear();
    st.route.set(route);
    st.menu.set(None);
    if same_page {
        scroll_top();
    }
}

pub fn go_back(state: AppState) {
    let mut st = state;
    let prev = st.back.write().pop();
    if let Some(prev) = prev {
        let cur = st.route.peek().clone();
        let same_page = cur.key() == prev.key();
        st.forward.write().push(cur);
        if same_page {
            scroll_top();
        } else {
            NAV_BACK.with(|b| b.set(true));
        }
        st.route.set(prev);
    }
}

pub fn go_forward(state: AppState) {
    let mut st = state;
    let next = st.forward.write().pop();
    if let Some(next) = next {
        let cur = st.route.peek().clone();
        let same_page = cur.key() == next.key();
        st.back.write().push(cur);
        st.route.set(next);
        if same_page {
            scroll_top();
        }
    }
}

thread_local! {
    /// Переход сделан кнопкой «Назад» — страница уезжает «наружу», а не вглубь.
    static NAV_BACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Забрать подсказку «Назад» (её читает стек страниц при смене маршрута).
pub fn take_nav_back() -> bool {
    NAV_BACK.with(|b| b.replace(false))
}

/// Прокрутить текущую страницу к началу. У каждой страницы своя прокрутка,
/// поэтому новая страница и так открывается сверху — это нужно, только когда
/// страница остаётся той же (вкладки настроек).
pub fn scroll_top() {
    document::eval("document.querySelector('.page-wrap.current')?.scrollTo({top: 0, behavior: 'instant'});");
}

// ─────────────────────────────── воспроизведение ───────────────────────────────

pub fn play_list(state: AppState, tracks: Vec<Track>, start: usize, label: &str) {
    if tracks.is_empty() {
        return;
    }
    let (shuffle, skip_previews, replace) = {
        let s = state.settings.peek();
        (s.shuffle, s.skip_previews, s.yt_replace_previews)
    };
    let (tracks, start) = playable_from(tracks, start, skip_previews, replace);
    if tracks.is_empty() {
        toast(state, ToastKind::Error, "Эти треки недоступны для воспроизведения");
        return;
    }
    let mut q = state.queue;
    q.write().set(tracks, start, shuffle, label);
    start_current(state, 0, true);
}

/// Убирает из списка недоступные треки (и превью, если их пропускаем) и
/// пересчитывает индекс выбранного по позиции, а не по сдвигу: иначе
/// недоступный трек выше выбранного включал бы соседний снизу. Выбранное
/// превью остаётся; если сам выбранный недоступен — следующий доступный.
/// С заменой с YouTube (`replace`) превью и недоступные треки остаются в очереди:
/// они сыграют полной версией с YouTube.
fn playable_from(tracks: Vec<Track>, start: usize, skip_previews: bool, replace: bool) -> (Vec<Track>, usize) {
    let start = start.min(tracks.len().saturating_sub(1));
    let mut out = Vec::with_capacity(tracks.len());
    let mut new_start = None;
    for (i, t) in tracks.into_iter().enumerate() {
        if !replace && (t.is_blocked() || (skip_previews && t.is_preview() && i != start)) {
            continue;
        }
        if i >= start && new_start.is_none() {
            new_start = Some(out.len());
        }
        out.push(t);
    }
    (out, new_start.unwrap_or(0))
}

pub fn play_single(state: AppState, track: Track) {
    play_list(state, vec![track], 0, "Трек");
}

pub fn start_current(state: AppState, start_ms: u64, autoplay: bool) {
    let Some(track) = state.queue.peek().current().cloned() else { return };
    let mut s = state;
    let id = *s.load_id.peek() + 1;
    s.load_id.set(id);
    s.now.set(Some(track.clone()));
    s.pstate.set(PlaybackState::Loading);
    s.position.set(start_ms);
    s.duration.set(track.duration_ms);
    s.buffered.set(0.0);
    s.restore_pos.set(None);
    s.preview.set(false);
    s.library.write().push_history(&track);

    if let Some(url) = track.artwork.as_deref().map(|u| crate::model::sc_art_size(u, ArtSize::Large)) {
        let _ = svc().rt.spawn(async move {
            let _ = svc().media.image_bytes(&url).await;
        });
    }

    load_lyrics(state, &track);
    load_accent(state, &track);

    spawn_forever(async move {
        let res = resolve(state, &track).await;
        if *s.load_id.peek() != id {
            return;
        }
        match res {
            Ok((spec, preview)) => {
                s.preview.set(preview);
                if preview {
                    toast(state, ToastKind::Info, "SoundCloud отдаёт только 30-секундное превью этого трека");
                }
                svc().player.load(id, spec, start_ms, autoplay, track.duration_ms);
            }
            Err(e) => {
                s.pstate.set(PlaybackState::Error);
                toast(state, ToastKind::Error, format!("«{}»: {e}", track.title));
                skip_after_error(state, id);
            }
        }
    });
}

async fn resolve(state: AppState, track: &Track) -> Result<(MediaSpec, bool), String> {
    if let Some(p) = &track.path {
        return Ok((MediaSpec::File(p.into()), false));
    }
    let pre = state.preloaded.peek().clone();
    if pre.track_id == track.id && pre.at.is_some_and(|t| t.elapsed() < Duration::from_secs(300)) {
        if let Some(spec) = pre.spec {
            return Ok((spec, pre.preview));
        }
    }
    let (quality, replace) = {
        let s = state.settings.peek();
        (s.quality, s.yt_replace_previews)
    };
    let res = bg(fetch_stream(track.clone(), quality, replace)).await.unwrap_or_else(|| Err("внутренняя ошибка".into()));
    res.map(|(spec, preview, from_yt)| {
        if from_yt {
            toast(state, ToastKind::Info, "Вместо превью SoundCloud играет полная версия с YouTube");
        }
        (spec, preview)
    })
}

/// Поток для трека: трек YouTube; превью или недоступный трек SoundCloud — полной
/// версией с YouTube (если включено); иначе — сам SoundCloud.
/// Возвращает (спецификация, это 30-секундное превью, поток взят с YouTube вместо SoundCloud).
async fn fetch_stream(t: Track, quality: StreamQuality, replace: bool) -> Result<(MediaSpec, bool, bool), String> {
    if crate::youtube::video_id(&t).is_some() {
        return svc().yt.stream(&t).await.map(|spec| (spec, false, false));
    }
    let tried = replace && (t.is_preview() || t.is_blocked());
    if tried {
        match svc().yt.replacement(&t).await {
            Ok(spec) => return Ok((spec, false, true)),
            Err(e) => tracing::info!("замена с YouTube для «{}»: {e}", t.title),
        }
    }
    let (spec, preview) = svc().sc.stream(&t, quality).await?;
    // превью выяснилось только по самому потоку (API сайта)
    if preview && replace && !tried {
        if let Ok(spec) = svc().yt.replacement(&t).await {
            return Ok((spec, false, true));
        }
    }
    Ok((spec, preview, false))
}

pub fn skip_after_error(state: AppState, failed_id: u64) {
    let mut errs = state.errors_in_row;
    let n = *errs.peek() + 1;
    errs.set(n);
    if n >= 4 {
        toast(state, ToastKind::Error, "Несколько треков подряд не воспроизводятся — остановка");
        errs.set(0);
        return;
    }
    let has_next = {
        let q = state.queue.peek();
        q.index.is_some_and(|i| i + 1 < q.tracks.len())
    };
    if has_next {
        spawn_forever(async move {
            tokio::time::sleep(Duration::from_millis(1400)).await;
            if *state.load_id.peek() == failed_id {
                next(state, false);
            }
        });
    }
}

pub fn next(state: AppState, manual: bool) {
    let repeat = state.settings.peek().repeat;
    let (cur, len) = {
        let q = state.queue.peek();
        (q.index, q.tracks.len())
    };
    let Some(cur) = cur else { return };
    if !manual && repeat == RepeatMode::One {
        svc().player.seek(0);
        svc().player.play();
        return;
    }
    let mut q = state.queue;
    if cur + 1 < len {
        q.write().index = Some(cur + 1);
        start_current(state, 0, true);
    } else if repeat == RepeatMode::All && len > 0 {
        q.write().index = Some(0);
        start_current(state, 0, true);
    } else if state.settings.peek().autoplay_related {
        autoplay_related(state);
    } else if !manual {
        let mut p = state.pstate;
        p.set(PlaybackState::Ended);
    }
}

pub fn prev(state: AppState) {
    let restart_after = (state.settings.peek().prev_restart_s * 1000.0) as u64;
    if *state.position.peek() > restart_after {
        seek(state, 0);
        return;
    }
    let cur = state.queue.peek().index;
    match cur {
        Some(i) if i > 0 => {
            let mut q = state.queue;
            q.write().index = Some(i - 1);
            start_current(state, 0, true);
        }
        _ => seek(state, 0),
    }
}

pub fn jump_to(state: AppState, index: usize) {
    let mut q = state.queue;
    if index < q.peek().tracks.len() {
        q.write().index = Some(index);
        start_current(state, 0, true);
    }
}

fn autoplay_related(state: AppState) {
    let seed = state.queue.peek().current().cloned();
    let Some(seed) = seed else { return };
    let Some(urn) = seed.urn.clone() else {
        let mut p = state.pstate;
        p.set(PlaybackState::Ended);
        return;
    };
    if !svc().sc.configured() {
        return;
    }
    spawn_forever(async move {
        let res = bg(async move { svc().sc.related(&urn).await }).await;
        match res {
            Some(Ok(page)) => {
                let known: std::collections::HashSet<String> = {
                    let q = state.queue.peek();
                    let lib = state.library.peek();
                    q.tracks.iter().map(|t| t.id.clone()).chain(lib.history.iter().take(40).map(|t| t.id.clone())).collect()
                };
                let fresh: Vec<Track> = page.items.into_iter().filter(|t| !known.contains(&t.id) && !t.is_blocked()).take(15).collect();
                if fresh.is_empty() {
                    let mut p = state.pstate;
                    p.set(PlaybackState::Ended);
                    return;
                }
                let mut q = state.queue;
                {
                    let mut w = q.write();
                    let next_idx = w.index.map(|i| i + 1).unwrap_or(0);
                    w.tracks.extend(fresh);
                    w.index = Some(next_idx.min(w.tracks.len() - 1));
                }
                toast(state, ToastKind::Info, format!("Автовоспроизведение: похожие на «{}»", seed.title));
                start_current(state, 0, true);
            }
            Some(Err(e)) => toast(state, ToastKind::Error, e),
            None => {}
        }
    });
}

pub fn toggle_play(state: AppState) {
    let p = *state.pstate.peek();
    // трек из прошлой сессии показан «на паузе», но в движок ещё не загружен:
    // «играть» должно загрузить его с сохранённой позиции, а не слать «продолжить»
    let restored = state.restore_pos.peek().is_some();
    let loaded = state.now.peek().is_some() && !restored && !matches!(p, PlaybackState::Idle | PlaybackState::Error);
    if !loaded {
        if state.queue.peek().current().is_some() {
            let pos = state.restore_pos.peek().unwrap_or(0);
            start_current(state, pos, true);
        }
        return;
    }
    if p == PlaybackState::Ended {
        seek(state, 0);
        svc().player.play();
        return;
    }
    svc().player.toggle();
}

pub fn seek(state: AppState, ms: u64) {
    let mut pos = state.position;
    let dur = *state.duration.peek();
    let ms = if dur > 0 { ms.min(dur.saturating_sub(250)) } else { ms };
    if state.restore_pos.peek().is_some() {
        let mut r = state.restore_pos;
        r.set(Some(ms));
        pos.set(ms);
        return;
    }
    svc().player.seek(ms);
    pos.set(ms);
}

pub fn seek_rel(state: AppState, delta_ms: i64) {
    let pos = *state.position.peek() as i64;
    seek(state, (pos + delta_ms).max(0) as u64);
}

pub fn set_volume(state: AppState, v: f32) {
    let mut s = state.settings;
    let mut w = s.write();
    w.volume = v.clamp(0.0, w.max_volume);
    if w.volume > 0.0 {
        w.muted = false;
    }
}

/// Открыть или закрыть панель текста рукой пользователя: закрытие запоминается,
/// чтобы автооткрытие на следующем треке не возвращало панель.
pub fn set_lyrics_open(state: AppState, open: bool) {
    let mut s = state.settings;
    let mut w = s.write();
    w.lyrics_open = open;
    w.lyrics_dismissed = !open;
}

pub fn toggle_lyrics(state: AppState) {
    let open = !state.settings.peek().lyrics_open;
    set_lyrics_open(state, open);
}

pub fn toggle_mute(state: AppState) {
    let mut s = state.settings;
    let m = s.peek().muted;
    s.write().muted = !m;
}

/// Свернуть нижний плеер в островок в заголовке окна (или развернуть обратно).
/// С анимациями показанный сейчас элемент сначала доигрывает уход
/// (`player_switching`), и только потом его сменяет второй.
pub fn set_player_collapsed(state: AppState, collapsed: bool) {
    let mut st = state;
    if st.settings.peek().player_collapsed == collapsed || *st.player_switching.peek() {
        return;
    }
    // с островком на рабочем столе главное окно стекает каплей в островок и
    // прячется совсем — управлять музыкой можно с островка поверх всех окон
    if collapsed && st.settings.peek().island_desktop {
        super::desk_island::fly_out(state);
        return;
    }
    let (animate, k) = {
        let s = st.settings.peek();
        (s.animations && !s.low_power, s.anim_speed.clamp(0.05, 3.0))
    };
    if !animate {
        st.settings.write().player_collapsed = collapsed;
        return;
    }
    st.player_switching.set(true);
    let leave_ms = if collapsed { 230.0 } else { 170.0 };
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_millis((leave_ms * k) as u64)).await;
        let mut st = st;
        st.settings.write().player_collapsed = collapsed;
        st.player_switching.set(false);
    });
}

pub fn toggle_shuffle(state: AppState) {
    let mut s = state.settings;
    let on = !s.peek().shuffle;
    s.write().shuffle = on;
    let mut q = state.queue;
    if on {
        q.write().shuffle_on();
    } else {
        q.write().shuffle_off();
    }
    toast(state, ToastKind::Info, if on { "Перемешивание включено" } else { "Перемешивание выключено" });
}

pub fn cycle_repeat(state: AppState) {
    let mut s = state.settings;
    let r = s.peek().repeat.next();
    s.write().repeat = r;
    let label = match r {
        RepeatMode::Off => "Повтор выключен",
        RepeatMode::All => "Повтор очереди",
        RepeatMode::One => "Повтор трека",
    };
    toast(state, ToastKind::Info, label);
}

// ─────────────────────────────── очередь и библиотека ───────────────────────────────

pub fn add_next(state: AppState, t: Track) {
    let mut q = state.queue;
    let was_empty = q.peek().index.is_none();
    q.write().insert_next(t);
    toast(state, ToastKind::Success, "Будет следующим");
    if was_empty {
        start_current(state, 0, true);
    }
}

pub fn add_to_queue(state: AppState, t: Track) {
    let mut q = state.queue;
    let was_empty = q.peek().index.is_none();
    q.write().append(t);
    toast(state, ToastKind::Success, "Добавлено в очередь");
    if was_empty {
        start_current(state, 0, true);
    }
}

pub fn toggle_like(state: AppState, t: &Track) {
    let mut lib = state.library;
    let added = lib.write().toggle_favorite(t);
    toast(state, ToastKind::Success, if added { "Добавлено в избранное" } else { "Убрано из избранного" });
}

pub fn open_on_soundcloud(state: AppState, t: &Track) {
    if let Some(url) = &t.permalink {
        if open::that(url).is_err() {
            toast(state, ToastKind::Error, "Не удалось открыть браузер");
        }
    } else if let Some(p) = &t.path {
        if let Some(dir) = std::path::Path::new(p).parent() {
            let _ = open::that(dir);
        }
    }
}

// ─────────────────────────────── тексты и акцент ───────────────────────────────

pub fn load_lyrics(state: AppState, track: &Track) {
    let mut ly = state.lyrics;
    let mut off = state.lyrics_offset;
    if !state.settings.peek().lyrics_enabled {
        ly.set(LyricsState::Idle);
        return;
    }
    ly.set(LyricsState::Loading);
    off.set(svc().lyrics.offset(&track.id));
    let t = track.clone();
    let id = track.id.clone();
    spawn_forever(async move {
        let res = bg(async move { svc().lyrics.for_track(&t).await }).await;
        if state.now.peek().as_ref().map(|t| t.id.as_str()) != Some(id.as_str()) {
            return;
        }
        let found = matches!(res, Some(Ok(Some(_))));
        ly.set(match res {
            Some(Ok(Some(l))) => LyricsState::Found(l),
            Some(Ok(None)) => LyricsState::NotFound,
            Some(Err(e)) => LyricsState::Failed(e),
            None => LyricsState::Failed("внутренняя ошибка".into()),
        });
        let mut s = state.settings;
        // закрытую вручную панель не открываем — до тех пор, пока её не откроют сами
        if found && s.peek().lyrics_auto_open && !s.peek().lyrics_open && !s.peek().lyrics_dismissed {
            s.write().lyrics_open = true;
        }
    });
}

pub fn set_lyrics_offset(state: AppState, delta: i32) {
    let mut off = state.lyrics_offset;
    let v = (*off.peek() + delta).clamp(-30_000, 30_000);
    off.set(v);
    if let Some(t) = state.now.peek().as_ref() {
        svc().lyrics.set_offset(&t.id, v);
    }
}

fn accent_key(t: &Track) -> Option<String> {
    if let Some(p) = &t.path {
        return t.local_cover.then(|| format!("file:{p}"));
    }
    t.artwork.as_deref().map(|u| crate::model::sc_art_size(u, ArtSize::Medium))
}

pub fn load_accent(state: AppState, track: &Track) {
    let acc = state.accent;
    if !state.settings.peek().dynamic_accent {
        if acc.peek().is_some() {
            fade_accent(state, None);
        }
        return;
    }
    let Some(key) = accent_key(track) else {
        fade_accent(state, None);
        return;
    };
    let id = track.id.clone();
    spawn_forever(async move {
        let c = bg(async move { svc().media.accent_for(&key).await }).await.flatten();
        if state.now.peek().as_ref().map(|t| t.id.as_str()) == Some(id.as_str()) {
            fade_accent(state, c);
        }
    });
}

fn parse_hex(s: &str) -> Option<(f32, f32, f32)> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(f32::from);
    Some((c(0)?, c(2)?, c(4)?))
}

static ACCENT_GEN: AtomicU64 = AtomicU64::new(0);

/// Плавно перекрасить акцент к цвету обложки (`None` — к цвету темы):
/// ~0,7 с промежуточными оттенками вместо резкой смены.
pub fn fade_accent(state: AppState, target: Option<String>) {
    let mut acc = state.accent;
    let theme_accent = state.theme.peek().colors.accent.clone();
    let from = acc.peek().clone().unwrap_or_else(|| theme_accent.clone());
    let to = target.clone().unwrap_or_else(|| theme_accent.clone());
    let animate = {
        let s = state.settings.peek();
        s.animations && !s.low_power
    };
    let gen = ACCENT_GEN.fetch_add(1, Ordering::Relaxed) + 1;
    let (Some(a), Some(b)) = (parse_hex(&from), parse_hex(&to)) else {
        acc.set(target);
        return;
    };
    if !animate || from.eq_ignore_ascii_case(&to) {
        acc.set(target);
        return;
    }
    let k = state.settings.peek().anim_speed.clamp(0.25, 3.0);
    let steps = 16u32;
    let step = Duration::from_millis((700.0 * k / steps as f32) as u64);
    spawn_forever(async move {
        for i in 1..=steps {
            tokio::time::sleep(step).await;
            if ACCENT_GEN.load(Ordering::Relaxed) != gen {
                return; // уже началась новая смена цвета
            }
            let t = i as f32 / steps as f32;
            let t = t * t * (3.0 - 2.0 * t); // мягкое начало и конец
            let mix = |x: f32, y: f32| (x + (y - x) * t).round() as u8;
            let mut acc = state.accent;
            if i == steps {
                acc.set(target.clone());
            } else {
                acc.set(Some(format!("#{:02x}{:02x}{:02x}", mix(a.0, b.0), mix(a.1, b.1), mix(a.2, b.2))));
            }
        }
    });
}

// ─────────────────────────────── предзагрузка и сессия ───────────────────────────────

pub fn maybe_preload(state: AppState) {
    let (preload_next, crossfade, repeat) = {
        let s = state.settings.peek();
        (s.preload_next, s.crossfade_s > 0.0, s.repeat)
    };
    // плавному переходу нужен заранее известный следующий трек
    if !preload_next && !crossfade {
        return;
    }
    let (pos, dur) = (*state.position.peek(), *state.duration.peek());
    if dur == 0 || dur.saturating_sub(pos) > 25_000 {
        return;
    }
    // «повтор трека» — следующим будет этот же трек, переход не нужен
    if repeat == RepeatMode::One {
        return;
    }
    let next = {
        let q = state.queue.peek();
        q.index.and_then(|i| q.tracks.get(i + 1).or(if repeat == RepeatMode::All && i + 1 == q.tracks.len() { q.tracks.first() } else { None })).cloned()
    };
    let Some(next) = next else { return };
    if state.preloaded.peek().track_id == next.id {
        return;
    }
    let mut pre = state.preloaded;
    pre.set(Preloaded { track_id: next.id.clone(), spec: None, preview: false, at: None });
    // локальный файл готовить не нужно — движку достаточно знать путь
    if let Some(path) = next.path.clone() {
        svc().player.preload(MediaSpec::File(path.into()));
        return;
    }
    spawn_forever(async move {
        if crate::youtube::video_id(&next).is_some() || svc().sc.configured() {
            let (quality, replace) = {
                let s = state.settings.peek();
                (s.quality, s.yt_replace_previews)
            };
            if let Some(Ok((spec, preview, _))) = bg(fetch_stream(next.clone(), quality, replace)).await {
                svc().player.preload(spec.clone());
                pre.set(Preloaded { track_id: next.id.clone(), spec: Some(spec), preview, at: Some(Instant::now()) });
            }
            if let Some(url) = next.artwork.as_deref().map(|u| crate::model::sc_art_size(u, ArtSize::Large)) {
                let _ = bg(async move { svc().media.image_bytes(&url).await }).await;
            }
        }
        if state.settings.peek().lyrics_enabled {
            let t = next.clone();
            let _ = bg(async move { svc().lyrics.for_track(&t).await }).await;
        }
    });
}

pub fn save_session(state: AppState) {
    let remember = state.settings.peek().remember_session;
    let session = if remember {
        let q = state.queue.peek();
        q.index.map(|i| SavedSession {
            queue: q.tracks.iter().take(500).cloned().collect(),
            index: i.min(499),
            position_ms: *state.position.peek(),
        })
    } else {
        None
    };
    std::thread::spawn(move || crate::library::save_session(session.as_ref()));
}

pub fn restore_session(state: AppState) {
    if !state.settings.peek().remember_session {
        return;
    }
    let Some(s) = crate::library::load_session() else { return };
    if s.queue.is_empty() {
        return;
    }
    let mut q = state.queue;
    q.write().set(s.queue, s.index, false, "Прошлая сессия");
    let cur = q.peek().current().cloned();
    if let Some(t) = cur {
        let mut st = state;
        st.now.set(Some(t.clone()));
        st.duration.set(t.duration_ms);
        st.position.set(s.position_ms);
        st.restore_pos.set(Some(s.position_ms));
        st.pstate.set(PlaybackState::Paused);
        load_lyrics(state, &t);
        load_accent(state, &t);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, access: &str) -> Track {
        Track { id: id.into(), access: (!access.is_empty()).then(|| access.to_string()), ..Default::default() }
    }

    #[test]
    fn blocked_tracks_above_do_not_shift_the_choice() {
        let list = vec![t("a", ""), t("b", "blocked"), t("c", ""), t("d", "")];
        let (q, i) = playable_from(list, 2, false, false);
        assert_eq!(q[i].id, "c");
        // выбран недоступный — играет следующий доступный
        let list = vec![t("a", ""), t("b", "blocked"), t("c", "")];
        let (q, i) = playable_from(list, 1, false, false);
        assert_eq!(q[i].id, "c");
        // превью выше пропускаются, выбранное превью остаётся
        let list = vec![t("p", "preview"), t("x", "blocked"), t("q", "preview"), t("z", "")];
        let (q, i) = playable_from(list, 2, true, false);
        assert_eq!((q.len(), q[i].id.as_str()), (2, "q"));
        // с заменой с YouTube ничего не выкидываем
        let list = vec![t("p", "preview"), t("x", "blocked"), t("q", "preview"), t("z", "")];
        let (q, i) = playable_from(list, 2, true, true);
        assert_eq!((q.len(), q[i].id.as_str()), (4, "q"));
    }
}
