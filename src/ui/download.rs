//! Скачивание текущего трека: кнопка в плеере → окно «Куда сохранить»
//! (с галочкой «По умолчанию» — тогда больше не спрашивать) → файл в папке.
//! Папку и вопрос можно поменять в настройках («Воспроизведение»).

use super::actions::*;
use super::icons::Icon;
use super::state::*;
use crate::model::Track;
use crate::services::svc;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use std::path::PathBuf;
use std::time::Duration;

/// Окно «Куда сохранить» открыто для этого трека.
pub static DL_DIALOG: GlobalSignal<Option<Track>> = Signal::global(|| None);
/// Окно доигрывает уход.
static DL_LEAVING: GlobalSignal<bool> = Signal::global(|| false);

/// Папка по умолчанию: из настроек или системная «Загрузки».
pub fn default_dir(st: AppState) -> PathBuf {
    st.settings
        .peek()
        .download_dir
        .clone()
        .filter(|d| !d.trim().is_empty())
        .map(PathBuf::from)
        .or_else(dirs::download_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Кнопка «Скачать» в плеере.
pub fn start_download(st: AppState) {
    let Some(track) = st.now.peek().clone() else {
        toast(st, ToastKind::Info, "Сначала включите трек");
        return;
    };
    if st.settings.peek().download_ask {
        *DL_LEAVING.write() = false;
        *DL_DIALOG.write() = Some(track);
    } else {
        let dir = default_dir(st);
        save(st, track, dir);
    }
}

fn close_dialog() {
    if DL_DIALOG.peek().is_none() || *DL_LEAVING.peek() {
        return;
    }
    *DL_LEAVING.write() = true;
    spawn_forever(async move {
        tokio::time::sleep(Duration::from_millis(220)).await;
        *DL_LEAVING.write() = false;
        *DL_DIALOG.write() = None;
    });
}

/// Скачать поток трека в папку.
fn save(st: AppState, track: Track, dir: PathBuf) {
    toast(st, ToastKind::Info, format!("Скачиваю «{}»…", track.title));
    spawn_forever(async move {
        let res = match resolve_stream(st, &track).await {
            Ok((spec, preview)) => {
                let (artist, title) = (track.artist.clone(), track.title.clone());
                let dir2 = dir.clone();
                let r = bg(async move {
                    let http = svc().player.http_client();
                    let (data, ext) = crate::audio::download::fetch_track(&http, &spec, |_| {}).await?;
                    tokio::fs::create_dir_all(&dir2).await.map_err(|e| format!("не удалось создать папку: {e}"))?;
                    // не перезаписываем: «… (2).mp3», «… (3).mp3»
                    let name = crate::audio::download::file_name(&artist, &title, ext);
                    let mut path = dir2.join(&name);
                    let mut n = 2;
                    while path.exists() {
                        let stem = name.trim_end_matches(&format!(".{ext}")).to_string();
                        path = dir2.join(format!("{stem} ({n}).{ext}"));
                        n += 1;
                    }
                    tokio::fs::write(&path, &data).await.map_err(|e| format!("не удалось записать файл: {e}"))?;
                    Ok::<_, String>(path)
                })
                .await
                .unwrap_or_else(|| Err("загрузка прервалась".into()));
                r.map(|p| (p, preview))
            }
            Err(e) => Err(e),
        };
        match res {
            Ok((path, preview)) => {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let note = if preview { " (доступно только 30-секундное превью)" } else { "" };
                toast(st, ToastKind::Success, format!("Сохранено: {name}{note}"));
            }
            Err(e) => toast(st, ToastKind::Error, format!("Не удалось скачать «{}»: {e}", track.title)),
        }
    });
}

/// Окно «Куда сохранить».
#[component]
pub fn DownloadDialog() -> Element {
    let st = app_state();
    // хуки — до раннего выхода: их порядок не должен меняться между отрисовками
    let mut dir = use_signal(|| default_dir(st));
    let mut remember = use_signal(|| false);
    let open = DL_DIALOG();
    // при каждом открытии — папка по умолчанию и снятая галочка
    let mut was_open = use_signal(|| false);
    if open.is_some() != *was_open.peek() {
        was_open.set(open.is_some());
        if open.is_some() {
            dir.set(default_dir(st));
            remember.set(false);
        }
    }
    let Some(track) = open else { return rsx! {} };
    let leaving = DL_LEAVING();
    let shown = dir.read().display().to_string();
    rsx! {
        div { class: if leaving { "dl-backdrop leaving" } else { "dl-backdrop" }, onclick: move |_| close_dialog() }
        div { class: if leaving { "dl-panel glass leaving" } else { "dl-panel glass" }, onclick: move |e| e.stop_propagation(),
            div { class: "dl-head",
                span { class: "dl-icon", Icon { name: "download", size: 20 } }
                div { class: "dl-titles",
                    div { class: "dl-title", "Куда сохранить трек?" }
                    div { class: "dl-track ellipsis", "{track.artist} — {track.title}" }
                }
            }
            div { class: "dl-path",
                Icon { name: "folder", size: 16 }
                span { class: "dl-path-t ellipsis", title: "{shown}", "{shown}" }
                button { class: "dl-browse",
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(d) = rfd::AsyncFileDialog::new().set_title("Папка для скачанных треков").pick_folder().await {
                                dir.set(d.path().to_path_buf());
                            }
                        });
                    },
                    "Обзор…"
                }
            }
            label { class: "dl-check",
                input { r#type: "checkbox", checked: remember(), onchange: move |e: FormEvent| remember.set(e.checked()) }
                span { class: "dl-box", Icon { name: "check", size: 12 } }
                span { "По умолчанию — сохранять сюда и больше не спрашивать" }
            }
            div { class: "dl-actions",
                button { class: "dl-btn", onclick: move |_| close_dialog(), "Отмена" }
                button { class: "dl-btn primary",
                    onclick: {
                        let track = track.clone();
                        move |_| {
                            let d = dir.peek().clone();
                            if *remember.peek() {
                                let mut s = st.settings;
                                let mut w = s.write();
                                w.download_dir = Some(d.display().to_string());
                                w.download_ask = false;
                            }
                            save(st, track.clone(), d);
                            close_dialog();
                        }
                    },
                    Icon { name: "download", size: 15 }
                    "Скачать"
                }
            }
        }
    }
}
