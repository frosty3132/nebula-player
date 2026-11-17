//! Синхронизация плейлистов Nebula ↔ SoundCloud.
//!
//! - Свои плейлисты SoundCloud появляются в Nebula и связываются с ними.
//! - Плейлист Nebula с треками SoundCloud выкладывается на SoundCloud (закрытым)
//!   и тоже связывается. Файлы с компьютера и треки YouTube остаются только в Nebula.
//! - Изменения треков, порядка и названия с любой стороны сводятся
//!   трёхсторонним слиянием (`crate::sc_sync`).
//! - Удаление связанного плейлиста в Nebula удаляет его и на SoundCloud;
//!   если его удалили на SoundCloud — в Nebula он остаётся, но отвязывается.
//!
//! Запускается после входа, через несколько секунд после правки плейлиста,
//! раз в 10 минут и по кнопке «Синхронизировать».

use std::collections::HashMap;
use std::time::{Duration, Instant};

use dioxus::prelude::*;

use super::actions::{bg, toast};
use super::state::{AppState, ToastKind};
use crate::library::LocalPlaylist;
use crate::model::{PlaylistInfo, Track};
use crate::paths::now_secs;
use crate::services::svc;

#[derive(Clone, Debug, PartialEq)]
pub enum SyncStatus {
    Off,
    Running,
    Done(u64),
    Failed(String),
}

pub static STATUS: GlobalSignal<SyncStatus> = Signal::global(|| SyncStatus::Off);

/// urn треков SoundCloud в плейлисте (только они уходят на SoundCloud).
pub fn sc_urns(tracks: &[Track]) -> Vec<String> {
    tracks.iter().filter(|t| t.id.starts_with("sc:")).filter_map(|t| t.urn.clone()).collect()
}

fn enabled(st: AppState) -> bool {
    st.settings.peek().sc_playlist_sync && st.sc_user.peek().as_ref().is_some_and(|u| !u.urn.is_empty())
}

/// Есть ли что отправить: связанный плейлист изменился с прошлой синхронизации
/// или появился новый плейлист с треками SoundCloud.
fn dirty(st: AppState) -> bool {
    st.library.peek().playlists.iter().any(|p| {
        if p.sc_off {
            return false;
        }
        let urns = sc_urns(&p.tracks);
        match &p.sc_urn {
            Some(_) => urns != p.sc_base || p.name.trim() != p.sc_base_title.trim(),
            None => !urns.is_empty(),
        }
    })
}

/// Что сделать с плейлистом Nebula по итогам похода в SoundCloud.
enum Outcome {
    Updated { id: String, link: Option<String>, seen_urns: Vec<String>, seen_name: String, ids: Vec<String>, title: String, modified: Option<String>, remote: Vec<Track> },
    Imported { info: PlaylistInfo, tracks: Vec<Track> },
    Created { id: String, info: PlaylistInfo, urns: Vec<String> },
    Gone { id: String, name: String },
}

/// Свести один плейлист Nebula с плейлистом SoundCloud (`link` — связываем впервые).
async fn sync_pair(p: &LocalPlaylist, info: &PlaylistInfo, link: bool) -> Result<Option<Outcome>, String> {
    let sc = &svc().sc;
    let urns = sc_urns(&p.tracks);
    let (base, base_title) = if link { (Vec::new(), String::new()) } else { (p.sc_base.clone(), p.sc_base_title.clone()) };
    let local_same = !link && urns == base && p.name.trim() == base_title.trim();
    if local_same && info.modified.is_some() && info.modified == p.sc_modified {
        return Ok(None); // нигде ничего не менялось
    }
    let remote_tracks = sc.playlist_tracks_all(&info.id).await?;
    let remote_urns = sc_urns(&remote_tracks);
    let m = crate::sc_sync::merge(&base, &base_title, &urns, &p.name, &remote_urns, &info.title);
    let mut modified = info.modified.clone();
    if m.push_tracks || m.push_title {
        modified = sc.update_playlist(&info.id, Some(&m.title), &m.ids).await?.and_then(|n| n.modified);
    }
    Ok(Some(Outcome::Updated {
        id: p.id.clone(),
        link: link.then(|| info.id.clone()),
        seen_urns: urns,
        seen_name: p.name.clone(),
        ids: m.ids,
        title: m.title,
        modified,
        remote: remote_tracks,
    }))
}

async fn plan(me: String, snapshot: Vec<LocalPlaylist>, ignored: Vec<String>) -> Result<(Vec<Outcome>, Vec<String>), String> {
    let sc = &svc().sc;
    let remote_all = sc.my_playlists_all().await?;
    let remote: Vec<PlaylistInfo> = remote_all.into_iter().filter(|p| p.user_urn.as_deref().is_none_or(|u| u == me)).collect();
    // плейлисты SoundCloud, у которых уже есть пара в Nebula
    let mut claimed: std::collections::HashSet<String> = snapshot.iter().filter_map(|p| p.sc_urn.clone()).collect();
    let mut out = Vec::new();
    let mut errors = Vec::new();

    for p in snapshot.iter().filter(|p| !p.sc_off) {
        let urns = sc_urns(&p.tracks);
        let res = match &p.sc_urn {
            Some(urn) => match remote.iter().find(|r| r.id == *urn) {
                Some(info) => sync_pair(p, info, false).await,
                None => Ok(Some(Outcome::Gone { id: p.id.clone(), name: p.name.clone() })),
            },
            None => {
                // первая связь: плейлист с тем же названием на SoundCloud — это он же, склеиваем
                let same = remote.iter().find(|r| !claimed.contains(&r.id) && !ignored.contains(&r.id) && r.title.trim().to_lowercase() == p.name.trim().to_lowercase());
                match same {
                    Some(info) => {
                        claimed.insert(info.id.clone());
                        sync_pair(p, info, true).await
                    }
                    None if !urns.is_empty() => sc.create_playlist(p.name.trim(), &urns).await.map(|info| {
                        claimed.insert(info.id.clone());
                        Some(Outcome::Created { id: p.id.clone(), info, urns })
                    }),
                    None => Ok(None),
                }
            }
        };
        match res {
            Ok(Some(o)) => out.push(o),
            Ok(None) => {}
            Err(e) => errors.push(format!("«{}»: {e}", p.name)),
        }
    }

    // свои плейлисты SoundCloud, которых ещё нет в Nebula
    for info in &remote {
        if claimed.contains(&info.id) || ignored.contains(&info.id) {
            continue;
        }
        match sc.playlist_tracks_all(&info.id).await {
            Ok(tracks) => out.push(Outcome::Imported { info: info.clone(), tracks }),
            Err(e) => errors.push(format!("«{}»: {e}", info.title)),
        }
    }
    Ok((out, errors))
}

/// Собрать треки плейлиста: порядок из слияния, данные треков — какие есть
/// (свои или с SoundCloud); файлы и YouTube остаются в конце.
fn rebuild(current: &[Track], remote: &[Track], ids: &[String]) -> Vec<Track> {
    let mut known: HashMap<String, Track> = HashMap::new();
    for t in remote.iter().chain(current.iter()) {
        if let Some(u) = t.urn.clone().filter(|_| t.id.starts_with("sc:")) {
            known.insert(u, t.clone());
        }
    }
    let mut out: Vec<Track> = ids.iter().filter_map(|u| known.get(u).cloned()).collect();
    out.extend(current.iter().filter(|t| !t.id.starts_with("sc:")).cloned());
    out
}

fn apply(st: AppState, outcomes: Vec<Outcome>) -> usize {
    let mut lib = st.library;
    let mut gone = Vec::new();
    let mut changed = 0;
    {
        let mut w = lib.write();
        for o in outcomes {
            match o {
                Outcome::Updated { id, link, seen_urns, seen_name, ids, title, modified, remote } => {
                    let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) else { continue };
                    if link.is_some() {
                        p.sc_urn = link;
                        changed += 1;
                    }
                    // пока ходили в SoundCloud, плейлист не трогали — ставим итог;
                    // иначе оставляем правку, а итог станет снимком (она уйдёт следующей синхронизацией)
                    if sc_urns(&p.tracks) == seen_urns && p.name == seen_name {
                        let tracks = rebuild(&p.tracks, &remote, &ids);
                        if tracks != p.tracks || p.name != title {
                            changed += 1;
                        }
                        p.tracks = tracks;
                        p.name = title.clone();
                    }
                    p.sc_base = ids;
                    p.sc_base_title = title;
                    p.sc_modified = modified;
                }
                Outcome::Imported { info, tracks } => {
                    if w.playlists.iter().any(|p| p.sc_urn.as_deref() == Some(info.id.as_str())) {
                        continue;
                    }
                    let id = format!("local-{}-{}", now_secs(), w.playlists.len());
                    w.playlists.push(LocalPlaylist {
                        id,
                        name: info.title.clone(),
                        sc_base: sc_urns(&tracks),
                        tracks,
                        created: now_secs(),
                        sc_urn: Some(info.id.clone()),
                        sc_base_title: info.title.clone(),
                        sc_modified: info.modified.clone(),
                        sc_off: false,
                    });
                    changed += 1;
                }
                Outcome::Created { id, info, urns } => {
                    let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) else { continue };
                    p.sc_base_title = p.name.clone();
                    p.sc_urn = Some(info.id);
                    p.sc_base = urns;
                    p.sc_modified = info.modified;
                    changed += 1;
                }
                Outcome::Gone { id, name } => {
                    if let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) {
                        p.sc_urn = None;
                        p.sc_base.clear();
                        p.sc_off = true;
                        gone.push(name);
                    }
                }
            }
        }
    }
    lib.peek().save();
    for name in gone {
        toast(st, ToastKind::Info, format!("Плейлист «{name}» удалён в SoundCloud — в Nebula он остался, без связи"));
    }
    changed
}

/// Запустить синхронизацию сейчас. `manual` — нажали кнопку: тогда сообщаем итог.
pub fn sync_now(st: AppState, manual: bool) {
    if !enabled(st) {
        *STATUS.write() = SyncStatus::Off;
        if manual {
            toast(st, ToastKind::Info, "Войдите в SoundCloud и включите синхронизацию плейлистов");
        }
        return;
    }
    if *STATUS.peek() == SyncStatus::Running {
        return;
    }
    let me = st.sc_user.peek().as_ref().map(|u| u.urn.clone()).unwrap_or_default();
    // вошли в другой аккаунт — старые связи к нему не относятся
    {
        let mut lib = st.library;
        let other = lib.peek().sc_sync_account.as_ref().is_some_and(|a| *a != me);
        if other || lib.peek().sc_sync_account.is_none() {
            let mut w = lib.write();
            if other {
                for p in w.playlists.iter_mut() {
                    p.sc_urn = None;
                    p.sc_base.clear();
                    p.sc_base_title.clear();
                    p.sc_modified = None;
                    p.sc_off = false;
                }
                w.sc_ignored.clear();
            }
            w.sc_sync_account = Some(me.clone());
        }
    }
    *STATUS.write() = SyncStatus::Running;
    let (snapshot, ignored) = {
        let l = st.library.peek();
        (l.playlists.clone(), l.sc_ignored.clone())
    };
    spawn(async move {
        let r = bg(plan(me, snapshot, ignored)).await.unwrap_or_else(|| Err("Синхронизация прервалась".into()));
        match r {
            Ok((outcomes, errors)) => {
                let changed = apply(st, outcomes);
                if let Some(e) = errors.first() {
                    tracing::warn!("синхронизация плейлистов: {}", errors.join("; "));
                    *STATUS.write() = SyncStatus::Failed(if errors.len() > 1 { format!("{e} и ещё {}", errors.len() - 1) } else { e.clone() });
                } else {
                    *STATUS.write() = SyncStatus::Done(now_secs());
                }
                if manual && errors.is_empty() {
                    toast(st, ToastKind::Success, if changed > 0 { "Плейлисты синхронизированы с SoundCloud" } else { "Плейлисты уже совпадают с SoundCloud" });
                }
            }
            Err(e) => {
                tracing::warn!("синхронизация плейлистов: {e}");
                *STATUS.write() = SyncStatus::Failed(e.clone());
                if manual {
                    toast(st, ToastKind::Error, format!("Синхронизация не удалась: {e}"));
                }
            }
        }
    });
}

/// Фоновый присмотр: вход, правки плейлистов, раз в 10 минут.
pub fn use_playlist_sync(st: AppState) {
    use_future(move || async move {
        let mut last_full: Option<Instant> = None;
        let mut dirty_since: Option<Instant> = None;
        let mut was_on = false;
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let on = enabled(st);
            if !on {
                if was_on {
                    *STATUS.write() = SyncStatus::Off;
                }
                was_on = false;
                last_full = None;
                continue;
            }
            if *STATUS.peek() == SyncStatus::Running {
                continue;
            }
            // после ошибки не долбим SoundCloud: следующая попытка через 2 минуты
            let failed = matches!(*STATUS.peek(), SyncStatus::Failed(_));
            let full_every = if failed { Duration::from_secs(120) } else { Duration::from_secs(600) };
            if !was_on || last_full.is_none_or(|t| t.elapsed() >= full_every) {
                was_on = true;
                last_full = Some(Instant::now());
                dirty_since = None;
                sync_now(st, false);
                continue;
            }
            if !failed && dirty(st) {
                let t = *dirty_since.get_or_insert_with(Instant::now);
                // ждём, пока правки утихнут, — не на каждый добавленный трек
                if t.elapsed() >= Duration::from_secs(4) {
                    dirty_since = None;
                    last_full = Some(Instant::now());
                    sync_now(st, false);
                }
            } else {
                dirty_since = None;
            }
        }
    });
}

/// Отвязать плейлист: дальше он живёт только в Nebula, на SoundCloud остаётся как был.
pub fn unlink(st: AppState, id: &str) {
    let mut lib = st.library;
    let mut w = lib.write();
    let urn = w.playlists.iter_mut().find(|p| p.id == id).and_then(|p| {
        p.sc_off = true;
        p.sc_base.clear();
        p.sc_modified = None;
        p.sc_urn.take()
    });
    if let Some(u) = urn {
        if !w.sc_ignored.contains(&u) {
            w.sc_ignored.push(u);
        }
    }
}

/// Снова связать (или впервые выложить) плейлист.
pub fn relink(st: AppState, id: &str) {
    {
        let mut lib = st.library;
        let mut w = lib.write();
        if let Some(p) = w.playlists.iter_mut().find(|p| p.id == id) {
            p.sc_off = false;
        }
    }
    let has_sc = st.library.peek().playlists.iter().find(|p| p.id == id).is_some_and(|p| !sc_urns(&p.tracks).is_empty());
    if !has_sc {
        toast(st, ToastKind::Info, "Плейлист появится в SoundCloud, когда в нём будет хотя бы один трек оттуда");
    }
    sync_now(st, false);
}

/// Удалить связанный плейлист и на SoundCloud.
pub fn delete_remote(st: AppState, urn: String, name: String) {
    spawn(async move {
        let r = bg(async move { svc().sc.delete_playlist(&urn).await }).await.unwrap_or_else(|| Err("прервано".into()));
        if let Err(e) = r {
            toast(st, ToastKind::Error, format!("«{name}» не удалился в SoundCloud: {e}"));
        }
    });
}

pub fn ago(secs: u64) -> String {
    let d = now_secs().saturating_sub(secs);
    if d < 60 {
        "только что".into()
    } else if d < 3600 {
        format!("{} мин назад", d / 60)
    } else {
        format!("{} ч назад", d / 3600)
    }
}

/// Плашка состояния синхронизации на странице «Плейлисты».
#[component]
pub fn SyncChip() -> Element {
    let st = super::state::app_state();
    let status = STATUS();
    if !st.settings.read().sc_playlist_sync || st.sc_user.read().is_none() {
        return rsx! {};
    }
    let (cls, text, title) = match &status {
        SyncStatus::Running => ("sync-chip run", "Синхронизация…".to_string(), "Сверяю плейлисты с SoundCloud".to_string()),
        SyncStatus::Done(t) => ("sync-chip ok", format!("SoundCloud · {}", ago(*t)), "Плейлисты связаны с SoundCloud. Нажмите, чтобы сверить сейчас".to_string()),
        SyncStatus::Failed(e) => ("sync-chip err", "Ошибка синхронизации".to_string(), format!("{e}. Нажмите, чтобы повторить")),
        SyncStatus::Off => ("sync-chip", "Синхронизировать".to_string(), "Связать плейлисты с SoundCloud".to_string()),
    };
    rsx! {
        button { class: "{cls}", title: "{title}", "data-el": "button",
            onclick: move |_| sync_now(st, true),
            super::icons::Icon { name: if matches!(status, SyncStatus::Failed(_)) { "warning" } else { "refresh" }, size: 15 }
            span { "{text}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(n: u32) -> Track {
        Track { id: format!("sc:soundcloud:tracks:{n}"), urn: Some(format!("soundcloud:tracks:{n}")), title: format!("t{n}"), ..Default::default() }
    }

    #[test]
    fn rebuild_keeps_files_and_uses_order() {
        let file = Track { id: "file:a.mp3".into(), path: Some("a.mp3".into()), ..Default::default() };
        let cur = vec![sc(1), file.clone(), sc(2)];
        let remote = vec![sc(3), sc(1)];
        let ids: Vec<String> = ["soundcloud:tracks:3", "soundcloud:tracks:1", "soundcloud:tracks:2"].iter().map(|s| s.to_string()).collect();
        let out = rebuild(&cur, &remote, &ids);
        let got: Vec<&str> = out.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(got, vec!["sc:soundcloud:tracks:3", "sc:soundcloud:tracks:1", "sc:soundcloud:tracks:2", "file:a.mp3"]);
    }
}
