//! Страница настроек с вкладками. Любое изменение применяется мгновенно
//! и сохраняется на диск автоматически (с задержкой, чтобы не писать файл
//! на каждое движение слайдера).

use super::actions::*;
use super::icons::Icon;
use super::sc_account::{self, LoginButton};
use super::state::*;
use super::theme_editor::{ElementsTab, LayoutTab, ThemesTab};
use super::widgets::*;
use crate::audio::{Player, EQ_FREQS};
use crate::model::{image_url, sc_art_size, ArtSize};
use crate::paths::paths;
use crate::services::svc;
use crate::settings::{LyricsAlign, Settings, StreamQuality, VolumeCurve, WindowEffect, EQ_PRESETS};
use crate::theme::Side;
use dioxus::prelude::*;

/// Короткий доступ к записи настроек: `upd(st, |s| s.limiter = v)`.
pub fn upd(st: AppState, f: impl FnOnce(&mut Settings)) {
    let mut s = st.settings;
    f(&mut s.write());
}

/// Страница настроек живёт, пока открыта любая её вкладка (ключ маршрута один),
/// поэтому подсветка в списке вкладок плавно переезжает, а содержимое вкладки
/// въезжает сверху или снизу — в зависимости от того, куда по списку перешли.
#[component]
pub fn SettingsPage(tab: SettingsTab) -> Element {
    let st = app_state();
    let index = SettingsTab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
    let dir = use_switch_dir(index, "sp-up", "sp-down", "sp-first");
    let body = match tab {
        SettingsTab::Sound => rsx! { SoundTab {} },
        SettingsTab::Playback => rsx! { PlaybackTab {} },
        SettingsTab::Lyrics => rsx! { LyricsTab {} },
        SettingsTab::Interface => rsx! { InterfaceTab {} },
        SettingsTab::Themes => rsx! { ThemesTab {} },
        SettingsTab::Layout => rsx! { LayoutTab {} },
        SettingsTab::Elements => rsx! { ElementsTab {} },
        SettingsTab::Account => rsx! { AccountTab {} },
        SettingsTab::Cache => rsx! { CacheTab {} },
        SettingsTab::Hotkeys => rsx! { HotkeysTab {} },
        SettingsTab::About => rsx! { AboutTab {} },
    };
    // обычные вкладки — сетка карточек (bento) в две колонки, если хватает
    // ширины; редакторы темы, раскладки и элементов остаются во всю ширину
    let grid = if matches!(tab, SettingsTab::Themes | SettingsTab::Layout | SettingsTab::Elements) { "" } else { "bento" };
    // поиск по настройкам: пока в поле есть текст, вместо вкладки — результаты
    let mut query = use_signal(String::new);
    let q = query.read().clone();
    let hits = super::settings_index::search(&q);
    let searching = !q.trim().is_empty();
    // ключ на корне rsx-блока: смена вкладки пересоздаёт только содержимое
    let body = if searching {
        rsx! {
            div { key: "{SEARCH_KEY}", class: "set-pane sp-first",
                SearchResults { hits: hits.iter().map(|e| (e.tab, e.card, e.label, e.hint, e.whole_card)).collect::<Vec<_>>(), query: q.clone(),
                    onpick: move |_| query.set(String::new()) }
            }
        }
    } else {
        rsx! {
            div { key: "{tab:?}", class: "set-pane {grid} {dir}", {body} }
        }
    };
    rsx! {
        div { class: "page settings",
            div { class: "set-layout",
                nav { class: if searching { "set-tabs glass searching" } else { "set-tabs glass" }, "data-glide": ".set-tab.active",
                    span { class: "glider set-glider" }
                    h2 { class: "set-tabs-title", "Настройки" }
                    div { class: "set-search",
                        Icon { name: "search", size: 16 }
                        input {
                            class: "set-search-input",
                            r#type: "text",
                            placeholder: "Поиск настроек",
                            autocomplete: "off",
                            spellcheck: false,
                            value: "{q}",
                            oninput: move |e: FormEvent| query.set(e.value()),
                            onkeydown: move |e: KeyboardEvent| {
                                match e.key() {
                                    // Enter — сразу к первому результату
                                    Key::Enter => {
                                        let typed = query.peek().clone();
                                        if let Some(first) = super::settings_index::search(&typed).first() {
                                            jump_to_setting(st, first.tab, first.label, first.card, first.whole_card);
                                            query.set(String::new());
                                        }
                                    }
                                    Key::Escape if !query.peek().is_empty() => {
                                        e.stop_propagation();
                                        query.set(String::new());
                                    }
                                    _ => {}
                                }
                            },
                        }
                        if searching {
                            button { class: "set-search-clear", title: "Очистить", onclick: move |_| query.set(String::new()),
                                Icon { name: "close", size: 14 }
                            }
                        }
                    }
                    for t in SettingsTab::ALL {
                        {
                            let n = hits.iter().filter(|e| e.tab == t).count();
                            let class = match (t == tab && !searching, searching && n == 0) {
                                (true, _) => "set-tab active",
                                (false, true) => "set-tab dim",
                                _ => "set-tab",
                            };
                            rsx! {
                                button {
                                    key: "{t:?}",
                                    class: "{class}",
                                    onclick: move |_| {
                                        query.set(String::new());
                                        if *st.route.peek() != Route::Settings(t) {
                                            let mut r = st.route;
                                            r.set(Route::Settings(t));
                                            scroll_top();
                                        }
                                    },
                                    Icon { name: t.icon(), size: 18 }
                                    span { class: "set-tab-label", "{t.label()}" }
                                    if searching && n > 0 {
                                        span { class: "set-tab-count", "{n}" }
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "set-body", {body} }
            }
        }
    }
}

/// Перейти к настройке: открыть её вкладку, прокрутить к строке (или карточке)
/// и коротко подсветить её.
fn jump_to_setting(st: AppState, tab: SettingsTab, label: &str, card: &str, whole_card: bool) {
    if *st.route.peek() != Route::Settings(tab) {
        let mut r = st.route;
        r.set(Route::Settings(tab));
    }
    let label = serde_json::to_string(label).unwrap_or_default();
    let card = serde_json::to_string(card).unwrap_or_default();
    let sel = if whole_card { ".set-card-title" } else { ".set-title" };
    document::eval(&format!(
        r#"(async () => {{
  const want = {label}, card = {card};
  for (let i = 0; i < 40; i++) {{
    const pane = document.querySelector(".page-wrap.current .set-pane:not(.leaving)") || document;
    // строка может быть скрыта условием (фон «Картинка», раздел ключей) — тогда через секунду подсвечиваем её карточку
    const el = [...pane.querySelectorAll("{sel}")].find(e => e.textContent.trim() === want) || (i > 20 ? [...pane.querySelectorAll(".set-card-title")].find(e => e.textContent.trim() === card) : null);
    if (el) {{
      const box = el.closest(".set-row") || el.closest(".set-card") || el;
      box.scrollIntoView({{ block: "center", behavior: "smooth" }});
      box.classList.remove("set-found"); void box.offsetWidth; box.classList.add("set-found");
      setTimeout(() => box.classList.remove("set-found"), 1900);
      return;
    }}
    await new Promise(r => setTimeout(r, 50));
  }}
}})();"#
    ));
}

/// Результаты поиска: название, путь «Вкладка · Карточка» и подсказка.
#[component]
fn SearchResults(hits: Vec<(SettingsTab, &'static str, &'static str, &'static str, bool)>, query: String, onpick: EventHandler<()>) -> Element {
    let st = app_state();
    rsx! {
        section { class: "set-card glass set-results",
            header { class: "set-card-head",
                span { class: "set-card-icon", Icon { name: "search", size: 18 } }
                div { class: "set-card-heading",
                    h3 { class: "set-card-title", "Поиск: «{query.trim()}»" }
                    p { class: "set-card-desc",
                        if hits.is_empty() { "Ничего не нашлось — попробуйте другое слово" } else { "Найдено: {hits.len()} · Enter — открыть первое" }
                    }
                }
            }
            div { class: "set-card-body",
                for (i , (tab , card , label , hint , whole)) in hits.into_iter().enumerate() {
                    button {
                        key: "{i}",
                        class: "set-hit",
                        onclick: move |_| {
                            onpick.call(());
                            jump_to_setting(st, tab, label, card, whole);
                        },
                        span { class: "set-hit-icon", Icon { name: tab.icon(), size: 16 } }
                        span { class: "set-hit-text",
                            span { class: "set-hit-label", "{label}" }
                            span { class: "set-hit-path",
                                if whole { "{tab.label()}" } else { "{tab.label()} · {card}" }
                                if !hint.is_empty() {
                                    " — {hint}"
                                }
                            }
                        }
                        span { class: "set-hit-go", Icon { name: "forward", size: 16 } }
                    }
                }
            }
        }
    }
}

/// Ключ панели результатов поиска (отличается от ключей вкладок).
const SEARCH_KEY: &str = "search";

fn opts(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

#[component]
fn SoundTab() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let devices = use_resource(|| async move { bg(async { tokio::task::spawn_blocking(Player::devices).await.unwrap_or_default() }).await.unwrap_or_default() });
    let mut dev_opts = vec![("".to_string(), "Системное по умолчанию".to_string())];
    if let Some(list) = devices.read().as_ref() {
        for d in list {
            dev_opts.push((d.id.clone(), if d.is_default { format!("{} (по умолчанию)", d.name) } else { d.name.clone() }));
        }
    }
    let device_now = st.device.read().clone();
    let max_pct = (s.max_volume * 100.0).round() as u32;
    rsx! {
        Card { title: "Громкость", icon: "volume", desc: "Уровень и поведение регулятора",
            Row { label: "Громкость", hint: "Выше 100% — цифровое усиление. Лимитер не даёт звуку хрипеть.",
                Slider { value: s.volume * 100.0, min: 0.0, max: s.max_volume * 100.0, step: 1.0, unit: "%", mark: Some(100.0),
                    oninput: move |v: f32| set_volume(st, v / 100.0) }
            }
            Row { label: "Максимум громкости",
                Seg { value: max_pct.to_string(), options: opts(&[("100", "100%"), ("150", "150%"), ("200", "200%"), ("300", "300%"), ("400", "400%")]),
                    onchange: move |v: String| upd(st, |s| { s.max_volume = v.parse::<f32>().unwrap_or(200.0) / 100.0; s.volume = s.volume.min(s.max_volume); }) }
            }
            Row { label: "Кривая громкости", hint: "«Естественная» точнее на малой громкости",
                Seg { value: format!("{:?}", s.volume_curve), options: opts(&[("Perceptual", "Естественная"), ("Linear", "Линейная")]),
                    onchange: move |v: String| upd(st, |s| s.volume_curve = if v == "Linear" { VolumeCurve::Linear } else { VolumeCurve::Perceptual }) }
            }
            Row { label: "Шаг громкости", hint: "Колесо мыши и стрелки ↑/↓",
                Slider { value: s.volume_step * 100.0, min: 1.0, max: 20.0, step: 1.0, unit: "%", oninput: move |v: f32| upd(st, |s| s.volume_step = v / 100.0) }
            }
        }
        Card { title: "Обработка", icon: "tune", desc: "Лимитер, баланс и плавность",
            Row { label: "Лимитер", hint: "Мягко ограничивает пики — обязателен при усилении выше 100%",
                Toggle { value: s.limiter, onchange: move |v| upd(st, |s| s.limiter = v) }
            }
            Row { label: "Плавность паузы и перемотки",
                Slider { value: s.fade_ms, min: 0.0, max: 800.0, step: 10.0, unit: " мс", oninput: move |v: f32| upd(st, |s| s.fade_ms = v) }
            }
            Row { label: "Баланс",
                Slider { value: s.balance, min: -1.0, max: 1.0, step: 0.05, decimals: 2, mark: Some(0.0), oninput: move |v: f32| upd(st, |s| s.balance = if v.abs() < 0.04 { 0.0 } else { v }) }
            }
            Row { label: "Моно", Toggle { value: s.mono, onchange: move |v| upd(st, |s| s.mono = v) } }
        }
        Card { title: "Эквалайзер", icon: "eq", desc: "10 полос и готовые пресеты", wide: true,
            Row { label: "Включить эквалайзер", Toggle { value: s.eq_enabled, onchange: move |v| upd(st, |s| s.eq_enabled = v) } }
            div { class: "chips",
                for (id , name , vals) in EQ_PRESETS.iter() {
                    button { key: "{id}", class: if s.eq_preset == *id { "chip active" } else { "chip" }, "data-el": "chip",
                        onclick: move |_| upd(st, |s| { s.eq_bands = *vals; s.eq_preset = id.to_string(); s.eq_enabled = true; }),
                        "{name}"
                    }
                }
            }
            div { class: if s.eq_enabled { "eq-bands wide" } else { "eq-bands wide off" },
                for i in 0..10usize {
                    div { key: "{i}", class: "eq-band",
                        span { class: "eq-db", "{s.eq_bands[i]:+.1}" }
                        input { r#type: "range", class: "eq-input", min: "-12", max: "12", step: "0.5", value: "{s.eq_bands[i]}",
                            oninput: move |e: FormEvent| if let Ok(v) = e.value().parse::<f32>() { upd(st, |s| { s.eq_bands[i] = v; s.eq_preset = "custom".into(); s.eq_enabled = true; }) } }
                        span { class: "eq-freq", if EQ_FREQS[i] >= 1000.0 { "{EQ_FREQS[i] / 1000.0}k" } else { "{EQ_FREQS[i]}" } }
                    }
                }
            }
            Row { label: "Предусиление", hint: "Уменьшите, если сильно подняли полосы",
                Slider { value: s.eq_preamp, min: -12.0, max: 6.0, step: 0.5, decimals: 1, unit: " дБ", oninput: move |v: f32| upd(st, |s| s.eq_preamp = v) }
            }
        }
        Card { title: "Устройство и качество", icon: "storage", desc: "Куда выводить звук и в каком качестве брать поток",
            Row { label: "Устройство вывода", hint: format!("Сейчас: {device_now}"),
                Select { value: s.output_device.clone().unwrap_or_default(), options: dev_opts,
                    onchange: move |v: String| upd(st, |s| s.output_device = if v.is_empty() { None } else { Some(v) }) }
            }
            Row { label: "Качество SoundCloud",
                Seg { value: format!("{:?}", s.quality), options: opts(&[("High", "AAC 160"), ("Low", "AAC 96"), ("Mp3", "MP3 128")]),
                    onchange: move |v: String| upd(st, |s| s.quality = match v.as_str() { "Low" => StreamQuality::Low, "Mp3" => StreamQuality::Mp3, _ => StreamQuality::High }) }
            }
            Row { label: "Буфер вперёд", hint: "Сколько секунд трека скачивать заранее (ленивая загрузка экономит трафик)",
                Slider { value: s.read_ahead_s as f32, min: 30.0, max: 900.0, step: 30.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.read_ahead_s = v as u32) }
            }
        }
    }
}

#[component]
fn PlaybackTab() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    rsx! {
        Card { title: "Переходы и очередь", icon: "play", desc: "Что звучит дальше и как треки сменяют друг друга",
            Row { label: "Плавный переход между треками", hint: "Как кроссфейд в Spotify: конец трека перетекает в начало следующего. 0 — выключено",
                Slider { value: s.crossfade_s, min: 0.0, max: 12.0, step: 1.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.crossfade_s = v) }
            }
            Row { label: "Похожие треки после очереди", hint: "Радио по мотивам последнего трека SoundCloud", Toggle { value: s.autoplay_related, onchange: move |v| upd(st, |s| s.autoplay_related = v) } }
            Row { label: "Предзагрузка следующего трека", hint: "Поток, обложка и текст готовятся за 25 секунд до конца", Toggle { value: s.preload_next, onchange: move |v| upd(st, |s| s.preload_next = v) } }
            Row { label: "Запоминать очередь и позицию", Toggle { value: s.remember_session, onchange: move |v| upd(st, |s| s.remember_session = v) } }
            Row { label: "Пропускать 30-секундные превью", hint: "Если не включена замена с YouTube", Toggle { value: s.skip_previews, onchange: move |v| upd(st, |s| s.skip_previews = v) } }
        }
        Card { title: "Управление", icon: "keyboard", desc: "Перемотка, кнопка «Назад» и списки",
            Row { label: "Номера треков в списках", Toggle { value: s.show_track_numbers, onchange: move |v| upd(st, |s| s.show_track_numbers = v) } }
            Row { label: "Шаг перемотки", hint: "Стрелки ← / →",
                Slider { value: s.seek_step_s, min: 1.0, max: 30.0, step: 1.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.seek_step_s = v) }
            }
            Row { label: "«Назад» перезапускает трек после", hint: "Иначе — переход к предыдущему",
                Slider { value: s.prev_restart_s, min: 0.0, max: 15.0, step: 1.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.prev_restart_s = v) }
            }
        }
        DownloadsCard {}
        YouTubeCard {}
    }
}

/// Скачивание треков: папка и вопрос «куда сохранить».
#[component]
fn DownloadsCard() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let dir = super::download::default_dir(st).display().to_string();
    rsx! {
        Card { title: "Скачивание треков", icon: "download", desc: "Кнопка «Скачать» в плеере сохраняет текущий трек в файл",
            Row { label: "Папка для треков", hint: "{dir}",
                button { class: "btn",
                    onclick: move |_| {
                        spawn(async move {
                            if let Some(d) = rfd::AsyncFileDialog::new().set_title("Папка для скачанных треков").pick_folder().await {
                                upd(st, |s| s.download_dir = Some(d.path().display().to_string()));
                            }
                        });
                    },
                    "Изменить…"
                }
            }
            Row { label: "Спрашивать, куда сохранить", hint: "Перед каждым скачиванием — окно с выбором папки и галочкой «По умолчанию»",
                Toggle { value: s.download_ask, onchange: move |v| upd(st, |s| s.download_ask = v) }
            }
        }
    }
}

/// YouTube через yt-dlp: замена превью SoundCloud и обслуживание yt-dlp.
#[component]
fn YouTubeCard() -> Element {
    let st = app_state();
    let replace = st.settings.read().yt_replace_previews;
    let mut busy = use_signal(|| false);
    let mut status = use_signal(|| None::<(bool, String)>);
    let update = move |_| {
        busy.set(true);
        status.set(Some((true, "Проверяю yt-dlp (при первом запуске он скачается, ~18 МБ)…".into())));
        spawn(async move {
            match bg(async { svc().yt.update().await }).await {
                Some(Ok(v)) => status.set(Some((true, format!("yt-dlp {v} готов к работе")))),
                Some(Err(e)) => status.set(Some((false, e))),
                None => {}
            }
            busy.set(false);
        });
    };
    rsx! {
        Card { title: "YouTube (yt-dlp)", icon: "radio", desc: "Полные версии вместо превью SoundCloud",
            Row { label: "Заменять превью SoundCloud полной версией", hint: "30-секундные превью и недоступные в регионе треки ищутся на YouTube и играют целиком",
                Toggle { value: replace, onchange: move |v| upd(st, |s| s.yt_replace_previews = v) }
            }
            Row { label: "yt-dlp", hint: "Скачивается сам при первом обращении и обновляется раз в неделю. Свою копию можно положить рядом с Nebula.exe",
                button { class: "btn ghost", "data-el": "button", disabled: *busy.read(), onclick: update, Icon { name: "refresh", size: 18 } "Проверить и обновить" }
            }
            if let Some((ok, text)) = status.read().clone() {
                div { class: if ok { "note ok" } else { "note err" }, "{text}" }
            }
        }
    }
}

#[component]
fn LyricsTab() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let side = st.theme.read().layout.lyrics_side;
    rsx! {
        Card { title: "Панель текста", icon: "lyrics", desc: "Тексты песен из LRCLIB",
            Row { label: "Показывать тексты", Toggle { value: s.lyrics_enabled, onchange: move |v| upd(st, |s| s.lyrics_enabled = v) } }
            Row { label: "Открывать панель, когда текст найден", Toggle { value: s.lyrics_auto_open, onchange: move |v| upd(st, |s| s.lyrics_auto_open = v) } }
            Row { label: "Сторона панели",
                Seg { value: format!("{side:?}"), options: opts(&[("Left", "Слева"), ("Right", "Справа")]),
                    onchange: move |v: String| { let mut t = st.theme; t.write().layout.lyrics_side = if v == "Left" { Side::Left } else { Side::Right }; } }
            }
            Row { label: "Ширина панели", Slider { value: s.lyrics_width, min: 260.0, max: 760.0, step: 10.0, unit: " px", oninput: move |v: f32| upd(st, |s| s.lyrics_width = v) } }
        }
        Card { title: "Оформление", icon: "brush", desc: "Шрифт, выравнивание и подсветка строк",
            Row { label: "Размер шрифта", Slider { value: s.lyrics_font, min: 14.0, max: 44.0, step: 1.0, unit: " px", oninput: move |v: f32| upd(st, |s| s.lyrics_font = v) } }
            Row { label: "Выравнивание",
                Seg { value: format!("{:?}", s.lyrics_align), options: opts(&[("Left", "Слева"), ("Center", "По центру"), ("Right", "Справа")]),
                    onchange: move |v: String| upd(st, |s| s.lyrics_align = match v.as_str() { "Left" => LyricsAlign::Left, "Right" => LyricsAlign::Right, _ => LyricsAlign::Center }) }
            }
            Row { label: "Караоке-заливка строки", Toggle { value: s.lyrics_karaoke, onchange: move |v| upd(st, |s| s.lyrics_karaoke = v) } }
            Row { label: "Размывать неактивные строки", Toggle { value: s.lyrics_blur, onchange: move |v| upd(st, |s| s.lyrics_blur = v) } }
        }
        Card { title: "Синхронизация", icon: "timer", desc: "Прокрутка и сдвиг текста относительно звука",
            Row { label: "Автопрокрутка", hint: "Пауза на 3 секунды, если вы листаете сами", Toggle { value: s.lyrics_autoscroll, onchange: move |v| upd(st, |s| s.lyrics_autoscroll = v) } }
            Row { label: "Показывать несинхронный текст", Toggle { value: s.lyrics_plain_fallback, onchange: move |v| upd(st, |s| s.lyrics_plain_fallback = v) } }
            Row { label: "Общий сдвиг", hint: "Добавляется к сдвигу конкретного трека",
                Slider { value: s.lyrics_offset_ms as f32 / 1000.0, min: -5.0, max: 5.0, step: 0.1, decimals: 1, unit: " с", mark: Some(0.0),
                    oninput: move |v: f32| upd(st, |s| s.lyrics_offset_ms = (v * 1000.0) as i32) }
            }
        }
    }
}

#[component]
fn InterfaceTab() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let win_alpha = st.theme.read().glass.window_opacity;
    rsx! {
        Card { title: "Окно", icon: "layout", desc: "Прозрачность, эффект Windows и масштаб",
            Row { label: "Прозрачность окна", hint: "Сквозь окно виден рабочий стол",
                Slider { value: win_alpha * 100.0, min: 20.0, max: 100.0, step: 1.0, unit: "%", oninput: move |v: f32| { let mut t = st.theme; t.write().glass.window_opacity = v / 100.0; } }
            }
            Row { label: "Эффект окна Windows", hint: "Mica и Acrylic работают в Windows 11; Acrylic может притормаживать при перетаскивании",
                Seg { value: format!("{:?}", s.window_effect), options: opts(&[("None", "Нет"), ("Mica", "Mica"), ("Acrylic", "Acrylic"), ("Blur", "Blur")]),
                    onchange: move |v: String| {
                        let e = match v.as_str() { "Mica" => WindowEffect::Mica, "Acrylic" => WindowEffect::Acrylic, "Blur" => WindowEffect::Blur, _ => WindowEffect::None };
                        upd(st, |s| s.window_effect = e);
                        // эффект виден только сквозь полупрозрачное окно: в непрозрачной теме
                        // (например, «Монохром») выбор раньше ничего не менял
                        if e != WindowEffect::None && st.theme.peek().glass.window_opacity > 0.95 {
                            let mut t = st.theme;
                            t.write().glass.window_opacity = 0.82;
                            toast(st, ToastKind::Info, "Прозрачность окна снижена до 82% — иначе эффект не виден");
                        }
                    } }
            }
            Row { label: "Масштаб интерфейса", hint: "Применяется, когда отпускаешь ползунок", Slider { value: s.ui_scale * 100.0, min: 70.0, max: 160.0, step: 5.0, unit: "%", commit_on_release: true, oninput: move |v: f32| upd(st, |s| s.ui_scale = v / 100.0) } }
            Row { label: "Поверх всех окон", Toggle { value: s.always_on_top, onchange: move |v| upd(st, |s| s.always_on_top = v) } }
            Row { label: "Островок на рабочем столе", hint: "Свёрнутый плеер — капля вверху основного монитора поверх всех окон: плеер с текстом при наведении, громкость колёсиком, поиск, плейлисты и эквалайзер. Окно Nebula на это время прячется",
                Toggle { value: s.island_desktop, onchange: move |v| upd(st, |s| s.island_desktop = v) }
            }
            Row { label: "Часы в «Сцене»", hint: "Оформление виджета часов; щелчок по самим часам тоже переключает",
                Seg { value: s.clock_style.clone(), options: super::stage::CLOCK_STYLES.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
                    onchange: move |v: String| upd(st, |s| s.clock_style = v) }
            }
            Row { label: "Сворачивать в трей при закрытии", hint: "Крестик прячет Nebula к часам у панели задач — музыка играет дальше. Вернуть окно или выйти — щелчком по значку",
                Toggle { value: s.close_to_tray, onchange: move |v| upd(st, |s| s.close_to_tray = v) }
            }
        }
        Card { title: "Анимации", icon: "sparkles", desc: "Переходы, скорость и заставка",
            Row { label: "Анимации", Toggle { value: s.animations, onchange: move |v| upd(st, |s| s.animations = v) } }
            Row { label: "Скорость анимаций", hint: "Меньше — быстрее",
                Slider { value: s.anim_speed, min: 0.25, max: 2.5, step: 0.05, decimals: 2, unit: "×", oninput: move |v: f32| upd(st, |s| s.anim_speed = v) }
            }
            Row { label: "Заставка при запуске", hint: "Логотип, затем интерфейс плавно собирается по частям. Пропустить — клик или любая клавиша",
                Toggle { value: s.intro_anim, onchange: move |v| upd(st, |s| s.intro_anim = v) }
            }
        }
        Card { title: "Производительность", icon: "storage", desc: "Нагрузка на процессор и видеокарту",
            Row { label: "Пауза фона, когда окно неактивно", hint: "Звёзды и видео замирают — ноль нагрузки в фоне", Toggle { value: s.pause_bg_unfocused, onchange: move |v| upd(st, |s| s.pause_bg_unfocused = v) } }
            Row { label: "Экономный режим", hint: "Меньше размытия, звёзд и эффектов — для слабых ПК и ноутбуков", Toggle { value: s.low_power, onchange: move |v| upd(st, |s| s.low_power = v) } }
        }
        Card { title: "Эффекты", icon: "palette", desc: "Цвет, пульсация, визуализатор и прокрутка",
            Row { label: "Цвет акцента из обложки", Toggle { value: s.dynamic_accent, onchange: move |v| {
                upd(st, |s| s.dynamic_accent = v);
                if let Some(t) = st.now.peek().clone() { load_accent(st, &t); }
            } } }
            Row { label: "Пульсация обложки в такт басу", Toggle { value: s.beat_glow, onchange: move |v| upd(st, |s| s.beat_glow = v) } }
            Row { label: "Эффект «рыбий глаз» при прокрутке", hint: "Треки и карточки у верхнего и нижнего края списка сжимаются и наклоняются, как в барабане",
                Toggle { value: s.fisheye, onchange: move |v| upd(st, |s| s.fisheye = v) }
            }
            Row { label: "Визуализатор на главной", hint: "Спектр как в cava внутри планеты; считается, только пока он на экране",
                Toggle { value: s.visualizer, onchange: move |v| upd(st, |s| s.visualizer = v) }
            }
        }
        Card { title: "Discord Rich Presence", icon: "radio", desc: "Статус «Слушает» в профиле Discord",
            p { class: "set-text", "Статус «Слушает» в Discord: название и исполнитель, обложка трека, полоса времени с текущей секундой и кнопка «Открыть в SoundCloud». Работает через встроенное приложение Nebula — ничего настраивать не нужно." }
            Row { label: "Включить Discord RPC", Toggle { value: s.discord_rpc_enabled, onchange: move |v| upd(st, |s| s.discord_rpc_enabled = v) } }
        }
    }
}

#[component]
fn AccountTab() -> Element {
    let st = app_state();
    let user = st.sc_user.read().clone();
    let web = svc().sc.web_mode();
    let mut dev_open = use_signal(|| svc().sc.has_dev_keys() && !svc().sc.web_mode());
    let dev_class = if dev_open() { "collapse-head open" } else { "collapse-head" };
    rsx! {
        Card { title: "Аккаунт SoundCloud", icon: "cloud", desc: "Вход, поиск, лайки и плейлисты",
            match user {
                Some(u) => {
                    let avatar = u.avatar.as_deref().map(|a| image_url(&sc_art_size(a, ArtSize::Medium)));
                    rsx! {
                        div { class: "sc-account",
                            div { class: "sc-avatar",
                                match avatar {
                                    Some(a) => rsx! { img { src: "{a}", alt: "", draggable: false } },
                                    None => rsx! { Icon { name: "person", size: 30 } },
                                }
                            }
                            div { class: "sc-acc-text",
                                div { class: "sc-acc-name ellipsis", "{u.username}" }
                                div { class: "set-hint", if web { "Вход через сайт SoundCloud" } else { "Вход через свои ключи API" } }
                            }
                            button { class: "btn ghost", "data-el": "button", onclick: move |_| sc_account::logout(st),
                                Icon { name: "logout", size: 18 } "Выйти"
                            }
                        }
                    }
                }
                None => rsx! {
                    p { class: "set-text", "Войдите так же, как на сайте: откроется окно со страницей входа SoundCloud — по паролю, через Google или Apple. Ключи разработчика и подписки не нужны." }
                    div { class: "set-actions", LoginButton { class: "big" } }
                    p { class: "set-hint sc-login-note", "Nebula сохраняет только токен сессии и только на этом компьютере. Повторный вход — в один клик." }
                },
            }
        }
        super::spotify_card::SpotifyCard {}
        Card { title: "Как это работает", icon: "info", desc: "Что видит и хранит Nebula",
            ul { class: "set-list",
                li { "Пароль вводится на странице SoundCloud — Nebula его не видит и не хранит." }
                li { "После входа работают поиск, жанры, похожие треки, ваши лайки, лента подписок и плейлисты." }
                li { "Nebula обращается к тому же API, что и сайт soundcloud.com. Это неофициальный способ: если SoundCloud изменит сайт, может понадобиться обновление Nebula." }
                li { "Треки с ограничениями правообладателя SoundCloud отдаёт только как 30-секундное превью — Nebula помечает их." }
            }
        }
        section { class: "set-card glass",
            button { class: "{dev_class}", onclick: move |_| dev_open.toggle(),
                span { class: "set-card-title", Icon { name: "code", size: 18 } "Свои ключи API (для разработчиков)" }
                Icon { name: "down", size: 18 }
            }
            if dev_open() {
                DevKeys {}
            }
        }
    }
}

/// Официальный API SoundCloud со своими ключами (нужна подписка Artist Pro).
#[component]
fn DevKeys() -> Element {
    let st = app_state();
    let (id0, secret0, redirect0) = svc().sc.credentials();
    let mut id = use_signal(|| id0.clone());
    let mut secret = use_signal(|| secret0.clone());
    let mut redirect = use_signal(|| redirect0.clone());
    let mut busy = use_signal(|| false);
    let mut status = use_signal(|| None::<(bool, String)>);
    let has_keys = svc().sc.has_dev_keys();
    let logged_in = st.sc_user.read().is_some();

    let save = move |_| {
        svc().sc.set_credentials(&id.peek(), &secret.peek(), &redirect.peek());
        let mut r = st.sc_ready;
        r.set(svc().sc.configured());
        let mut u = st.sc_user;
        u.set(svc().sc.user());
        if svc().sc.web_mode() {
            status.set(Some((true, "Ключи сохранены. Пока выполнен вход через сайт, используется он".into())));
            return;
        }
        busy.set(true);
        status.set(None);
        spawn(async move {
            let res = bg(async { svc().sc.token().await }).await;
            match res {
                Some(Ok(_)) => status.set(Some((true, "Подключение работает: токен приложения получен".into()))),
                Some(Err(e)) => status.set(Some((false, e))),
                None => {}
            }
            busy.set(false);
        });
    };
    let login = move |_| {
        busy.set(true);
        status.set(Some((true, "Откройте браузер и подтвердите вход в SoundCloud…".into())));
        spawn(async move {
            let res = bg(async { svc().sc.login().await }).await;
            match res {
                Some(Ok(u)) => {
                    status.set(Some((true, format!("Вы вошли как {}", u.username))));
                    let mut su = st.sc_user;
                    su.set(Some(u));
                }
                Some(Err(e)) => status.set(Some((false, e))),
                None => {}
            }
            busy.set(false);
        });
    };
    rsx! {
        p { class: "set-text",
            "Официальный API SoundCloud. Создайте приложение на "
            a { class: "link", onclick: move |_| { let _ = open::that("https://soundcloud.com/you/apps"); }, "soundcloud.com/you/apps" }
            " (для регистрации приложений SoundCloud требует подписку Artist Pro), укажите в нём Redirect URI из поля ниже и вставьте ключи сюда."
        }
        Row { label: "Client ID", TextInput { value: id(), placeholder: "например, aBcD…", onchange: move |v| id.set(v) } }
        Row { label: "Client Secret", TextInput { value: secret(), password: true, placeholder: "секретный ключ", onchange: move |v| secret.set(v) } }
        Row { label: "Redirect URI", hint: "Должен совпадать с указанным в приложении SoundCloud",
            TextInput { value: redirect(), onchange: move |v| redirect.set(v) }
        }
        div { class: "set-actions",
            button { class: "btn", disabled: *busy.read(), onclick: save, Icon { name: "save", size: 18 } "Сохранить и проверить" }
            if has_keys && !logged_in {
                button { class: "btn ghost", disabled: *busy.read(), onclick: login, Icon { name: "login", size: 18 } "Войти по ключам" }
            }
            if *busy.read() {
                Spinner {}
            }
        }
        if let Some((ok, text)) = status.read().clone() {
            div { class: if ok { "note ok" } else { "note err" }, "{text}" }
        }
    }
}

#[component]
fn CacheTab() -> Element {
    let st = app_state();
    let s = st.settings.read().clone();
    let mut size = use_resource(|| async move { bg(async { tokio::task::spawn_blocking(crate::media::cache_size_mb).await.unwrap_or(0.0) }).await.unwrap_or(0.0) });
    let used = size.read().unwrap_or(0.0);
    let p = paths();
    let cfg = p.config.display().to_string();
    let cache = p.cache.display().to_string();
    rsx! {
        Card { title: "Кэш", icon: "storage", desc: "Обложки и тексты на диске",
            p { class: "set-text", "Обложки, аватары и тексты сохраняются на диск и загружаются лениво — повторные открытия страниц не нагружают сеть и процессор." }
            Row { label: "Занято", hint: "Обложки + тексты", span { class: "big-num", "{used:.1} МБ" } }
            Row { label: "Лимит кэша обложек",
                Slider { value: s.image_cache_mb as f32, min: 50.0, max: 4000.0, step: 50.0, unit: " МБ", oninput: move |v: f32| upd(st, |s| s.image_cache_mb = v as u32) }
            }
            div { class: "set-actions",
                button { class: "btn ghost", onclick: move |_| size.restart(), Icon { name: "refresh", size: 18 } "Пересчитать" }
                button { class: "btn danger",
                    onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::Confirm { title: "Очистить кэш?".into(), text: "Обложки и тексты будут загружены заново по мере надобности.".into(), action: ConfirmAction::ClearCache })); },
                    Icon { name: "delete", size: 18 } "Очистить кэш"
                }
            }
        }
        Card { title: "Данные", icon: "folder", desc: "Где лежат настройки и библиотека",
            Row { label: "Настройки, темы, библиотека", span { class: "mono small", "{cfg}" } }
            Row { label: "Кэш", span { class: "mono small", "{cache}" } }
            Row { label: "Портативный режим", hint: "Пустой файл portable.txt рядом с программой — все данные хранятся в папке data рядом с exe",
                span { class: if p.portable { "badge ok" } else { "badge soft" }, if p.portable { "включён" } else { "выключен" } }
            }
            div { class: "set-actions",
                button { class: "btn ghost", onclick: move |_| { let _ = open::that(&paths().config); }, Icon { name: "folder", size: 18 } "Открыть папку настроек" }
                button { class: "btn danger",
                    onclick: move |_| { let mut m = st.modal; m.set(Some(Modal::Confirm { title: "Сбросить настройки?".into(), text: "Темы, библиотека и ключи SoundCloud не будут затронуты.".into(), action: ConfirmAction::ResetSettings })); },
                    Icon { name: "refresh", size: 18 } "Сбросить настройки"
                }
            }
        }
    }
}

#[component]
fn HotkeysTab() -> Element {
    let keys = [
        ("Пробел", "Пауза / воспроизведение"),
        ("← / →", "Перемотка назад / вперёд"),
        ("Ctrl + ← / →", "Предыдущий / следующий трек"),
        ("↑ / ↓", "Громкость"),
        ("M", "Без звука"),
        ("S", "Перемешивание"),
        ("R", "Режим повтора"),
        ("L", "Панель текста"),
        ("F", "Текст на весь экран"),
        ("F11", "Полноэкранное окно"),
        ("Q", "Очередь"),
        ("Ctrl + F или /", "Поиск"),
        ("Ctrl + E", "Режим редактирования интерфейса"),
        ("Ctrl + ,", "Настройки"),
        ("Esc", "Закрыть окно/панель/режим"),
        ("Медиаклавиши", "Play/Pause, следующий, предыдущий"),
    ];
    rsx! {
        Card { title: "Горячие клавиши", icon: "keyboard", desc: "Работают, когда окно Nebula в фокусе; медиаклавиши — всегда", wide: true,
            div { class: "hotkeys",
                for (k , v) in keys {
                    div { key: "{k}", class: "hk-row", kbd { "{k}" } span { "{v}" } }
                }
            }
        }
    }
}

#[component]
fn AboutTab() -> Element {
    rsx! {
        Card { title: "Nebula", icon: "info", desc: "О программе", wide: true,
            div { class: "about",
                span { class: "about-logo", dangerous_inner_html: super::icons::LOGO_SVG }
                div {
                    h2 { "Nebula {env!(\"CARGO_PKG_VERSION\")}" }
                    p { class: "set-text", "Галактический музыкальный плеер на Rust: Dioxus (UI на системном WebView2), symphonia (декодирование), cpal/WASAPI (вывод), собственный HLS-клиент и DSP." }
                }
            }
            ul { class: "set-list",
                li { "Музыка и метаданные — SoundCloud (официальный API). Каждый трек ссылается на свою страницу SoundCloud." }
                li { "Синхронные тексты — LRCLIB (lrclib.net), открытая база текстов." }
                li { "Локальные файлы: MP3, FLAC, WAV, OGG Vorbis, AAC/M4A, AIFF." }
            }
        }
    }
}
