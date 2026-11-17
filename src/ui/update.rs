//! Обновление из интерфейса: проверка при запуске, плашка «Доступна новая
//! версия» с кнопкой «Обновить», прогресс скачивания и перезапуск.

use dioxus::prelude::*;

use super::actions::{bg, toast};
use super::state::{AppState, ToastKind};
use super::icons::Icon;
use crate::updater::{self, Release};

#[derive(Clone, Debug, PartialEq)]
pub enum UpdState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading(Release, f32),
    Restarting(Release),
    Failed(Option<Release>, String),
}

pub static UPDATE: GlobalSignal<UpdState> = Signal::global(|| UpdState::Idle);
/// Плашку закрыли «Позже» — до следующего запуска не показываем.
static HIDDEN: GlobalSignal<bool> = Signal::global(|| false);

/// Проверить релизы. `manual` — нажали «Проверить сейчас»: тогда и «у вас
/// последняя версия», и ошибки сети показываем, а плашку открываем снова.
pub fn check_now(st: AppState, manual: bool) {
    if matches!(*UPDATE.peek(), UpdState::Checking | UpdState::Downloading(..) | UpdState::Restarting(_)) {
        return;
    }
    *UPDATE.write() = UpdState::Checking;
    if manual {
        *HIDDEN.write() = false;
    }
    spawn(async move {
        let r = bg(updater::check()).await.unwrap_or_else(|| Err("Проверка прервалась".into()));
        match r {
            Ok(Some(rel)) => {
                tracing::info!("доступна новая версия {}", rel.version);
                *UPDATE.write() = UpdState::Available(rel);
            }
            Ok(None) => {
                *UPDATE.write() = UpdState::UpToDate;
                if manual {
                    toast(st, ToastKind::Success, format!("У вас последняя версия — Nebula {}", updater::current_version()));
                }
            }
            Err(e) => {
                tracing::warn!("проверка обновлений: {e}");
                if manual {
                    *UPDATE.write() = UpdState::Failed(None, e);
                } else {
                    // при запуске молчим: нет сети — не повод для плашки
                    *UPDATE.write() = UpdState::Idle;
                }
            }
        }
    });
}

/// Скачать и поставить обновление, затем перезапуститься.
pub fn start_install(st: AppState) {
    let rel = match &*UPDATE.peek() {
        UpdState::Available(r) | UpdState::Failed(Some(r), _) => r.clone(),
        _ => return,
    };
    *UPDATE.write() = UpdState::Downloading(rel.clone(), 0.0);
    spawn(async move {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<f32>();
        let r2 = rel.clone();
        // задача стартует сразу, а этот цикл тем временем показывает прогресс
        let job = crate::services::svc().rt.spawn(async move { updater::install(&r2, move |p| { let _ = tx.send(p); }).await });
        // прогресс приходит из фонового потока
        let mut shown = 0.0f32;
        while let Some(p) = rx.recv().await {
            // кусков много — перерисовываем на каждый процент, не чаще
            if p - shown < 0.01 && p < 1.0 {
                continue;
            }
            shown = p;
            // чтение отпускаем до записи: иначе Dioxus падает (AlreadyBorrowed)
            let downloading = matches!(*UPDATE.peek(), UpdState::Downloading(..));
            if downloading {
                *UPDATE.write() = UpdState::Downloading(rel.clone(), p);
            }
        }
        match job.await.ok() {
            Some(Ok(exe)) => {
                *UPDATE.write() = UpdState::Restarting(rel.clone());
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
                match updater::relaunch(&exe) {
                    Ok(()) => super::chrome::quit_app(st),
                    Err(e) => *UPDATE.write() = UpdState::Failed(Some(rel), format!("Обновление установлено, но не перезапустилось: {e}. Откройте Nebula заново")),
                }
            }
            Some(Err(e)) => *UPDATE.write() = UpdState::Failed(Some(rel), e),
            None => *UPDATE.write() = UpdState::Failed(Some(rel), "Загрузка прервалась".into()),
        }
    });
}

fn mb(bytes: u64) -> String {
    format!("{:.1} МБ", bytes as f64 / 1_048_576.0)
}

/// Плашка обновления в правом верхнем углу.
#[component]
pub fn UpdateBanner() -> Element {
    let st = super::state::app_state();
    let state = UPDATE();
    let hidden = HIDDEN();
    let mut notes_open = use_signal(|| false);
    let show = !hidden && matches!(state, UpdState::Available(_) | UpdState::Downloading(..) | UpdState::Restarting(_) | UpdState::Failed(..));
    if !show {
        return rsx! {};
    }
    let cur = updater::current_version();
    rsx! {
        div { class: "upd-banner glass",
            match state.clone() {
                UpdState::Available(rel) => rsx! {
                    div { class: "upd-head",
                        span { class: "upd-ico", Icon { name: "download", size: 18 } }
                        div { class: "upd-meta",
                            div { class: "upd-title", "Доступна Nebula {rel.version}" }
                            div { class: "upd-sub", "Сейчас {cur} · {mb(rel.asset_size)} · без переустановки" }
                        }
                        button { class: "upd-x", title: "Позже", onclick: move |_| *HIDDEN.write() = true, Icon { name: "close", size: 14 } }
                    }
                    if !rel.notes.is_empty() {
                        button { class: "upd-notes-btn", onclick: move |_| notes_open.toggle(),
                            Icon { name: if notes_open() { "up" } else { "down" }, size: 14 }
                            "Что нового"
                        }
                        if notes_open() {
                            div { class: "upd-notes",
                                for (i, line) in rel.notes.lines().filter(|l| !l.trim().is_empty()).take(14).enumerate() {
                                    div { key: "{i}", class: if line.trim_start().starts_with('#') { "upd-note h" } else { "upd-note" },
                                        "{line.trim_start_matches(['#', ' ', '-', '*']).replace(\"**\", \"\")}"
                                    }
                                }
                            }
                        }
                    }
                    div { class: "upd-actions",
                        button { class: "upd-later", onclick: move |_| *HIDDEN.write() = true, "Позже" }
                        button { class: "upd-go", onclick: move |_| start_install(st), Icon { name: "download", size: 15 } "Обновить плеер" }
                    }
                },
                UpdState::Downloading(rel, p) => rsx! {
                    div { class: "upd-head",
                        span { class: "upd-ico spin", Icon { name: "refresh", size: 18 } }
                        div { class: "upd-meta",
                            div { class: "upd-title", "Обновляю до {rel.version}…" }
                            div { class: "upd-sub", "Скачано {(p * 100.0).round()}% · не закрывайте Nebula" }
                        }
                    }
                    div { class: "upd-bar", div { class: "upd-fill", style: "width: {(p * 100.0).clamp(2.0, 100.0)}%;" } }
                },
                UpdState::Restarting(rel) => rsx! {
                    div { class: "upd-head",
                        span { class: "upd-ico ok", Icon { name: "check", size: 18 } }
                        div { class: "upd-meta",
                            div { class: "upd-title", "Nebula {rel.version} установлена" }
                            div { class: "upd-sub", "Перезапускаю…" }
                        }
                    }
                    div { class: "upd-bar", div { class: "upd-fill", style: "width: 100%;" } }
                },
                UpdState::Failed(rel, err) => rsx! {
                    div { class: "upd-head",
                        span { class: "upd-ico err", Icon { name: "warning", size: 18 } }
                        div { class: "upd-meta",
                            div { class: "upd-title", if rel.is_some() { "Не получилось обновить" } else { "Не удалось проверить обновления" } }
                            div { class: "upd-sub wrap", "{err}" }
                        }
                        button { class: "upd-x", title: "Закрыть", onclick: move |_| *HIDDEN.write() = true, Icon { name: "close", size: 14 } }
                    }
                    if let Some(r) = rel {
                        div { class: "upd-actions",
                            button { class: "upd-later", onclick: { let page = r.page.clone(); move |_| { let _ = open::that(&page); } }, Icon { name: "external", size: 14 } "Страница релиза" }
                            button { class: "upd-go", onclick: move |_| start_install(st), Icon { name: "refresh", size: 15 } "Ещё раз" }
                        }
                    }
                },
                _ => rsx! {},
            }
        }
    }
}

/// Строка в настройках «О программе»: проверка при запуске и «Проверить сейчас».
#[component]
pub fn UpdateSettings() -> Element {
    let st = super::state::app_state();
    let on = st.settings.read().check_updates;
    let state = UPDATE();
    let status = match &state {
        UpdState::Idle => String::new(),
        UpdState::Checking => "Проверяю…".into(),
        UpdState::UpToDate => "Последняя версия".into(),
        UpdState::Available(r) => format!("Доступна {}", r.version),
        UpdState::Downloading(r, p) => format!("Обновляю до {} — {}%", r.version, (p * 100.0).round()),
        UpdState::Restarting(_) => "Перезапуск…".into(),
        UpdState::Failed(_, e) => e.clone(),
    };
    let available = matches!(state, UpdState::Available(_));
    rsx! {
        super::widgets::Row { label: "Проверять обновления при запуске", hint: "Nebula смотрит новые релизы на GitHub и предлагает обновиться одной кнопкой — без переустановки",
            super::widgets::Toggle { value: on, onchange: move |v| super::settings_page::upd(st, |s| s.check_updates = v) }
        }
        super::widgets::Row { label: "Версия {updater::current_version()}", hint: "{status}",
            if available {
                button { class: "btn primary", onclick: move |_| { *HIDDEN.write() = false; start_install(st); }, Icon { name: "download", size: 15 } "Обновить" }
            } else {
                button { class: "btn", disabled: matches!(state, UpdState::Checking | UpdState::Downloading(..) | UpdState::Restarting(_)),
                    onclick: move |_| check_now(st, true), Icon { name: "refresh", size: 15 } "Проверить сейчас" }
            }
        }
    }
}
