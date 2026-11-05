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
    // ключ на корне rsx-блока: смена вкладки пересоздаёт только содержимое
    let body = rsx! {
        div { key: "{tab:?}", class: "set-pane {dir}", {body} }
    };
    rsx! {
        div { class: "page settings",
            div { class: "set-layout",
                nav { class: "set-tabs glass", "data-glide": ".set-tab.active",
                    span { class: "glider set-glider" }
                    h2 { class: "set-tabs-title", "Настройки" }
                    for t in SettingsTab::ALL {
                        button {
                            key: "{t:?}",
                            class: if t == tab { "set-tab active" } else { "set-tab" },
                            onclick: move |_| {
                                if *st.route.peek() != Route::Settings(t) {
                                    let mut r = st.route;
                                    r.set(Route::Settings(t));
                                    scroll_top();
                                }
                            },
                            Icon { name: t.icon(), size: 18 }
                            span { "{t.label()}" }
                        }
                    }
                }
                div { class: "set-body", {body} }
            }
        }
    }
}

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
        Card { title: "Громкость", icon: "volume",
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
        Card { title: "Эквалайзер", icon: "eq",
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
        Card { title: "Устройство и качество", icon: "storage",
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
        Card { title: "Воспроизведение", icon: "play",
            Row { label: "Похожие треки после очереди", hint: "Радио по мотивам последнего трека SoundCloud", Toggle { value: s.autoplay_related, onchange: move |v| upd(st, |s| s.autoplay_related = v) } }
            Row { label: "Предзагрузка следующего трека", hint: "Поток, обложка и текст готовятся за 25 секунд до конца", Toggle { value: s.preload_next, onchange: move |v| upd(st, |s| s.preload_next = v) } }
            Row { label: "Запоминать очередь и позицию", Toggle { value: s.remember_session, onchange: move |v| upd(st, |s| s.remember_session = v) } }
            Row { label: "Пропускать 30-секундные превью", Toggle { value: s.skip_previews, onchange: move |v| upd(st, |s| s.skip_previews = v) } }
            Row { label: "Номера треков в списках", Toggle { value: s.show_track_numbers, onchange: move |v| upd(st, |s| s.show_track_numbers = v) } }
            Row { label: "Шаг перемотки", hint: "Стрелки ← / →",
                Slider { value: s.seek_step_s, min: 1.0, max: 30.0, step: 1.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.seek_step_s = v) }
            }
            Row { label: "«Назад» перезапускает трек после", hint: "Иначе — переход к предыдущему",
                Slider { value: s.prev_restart_s, min: 0.0, max: 15.0, step: 1.0, unit: " с", oninput: move |v: f32| upd(st, |s| s.prev_restart_s = v) }
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
        Card { title: "Тексты песен (LRCLIB)", icon: "lyrics",
            Row { label: "Показывать тексты", Toggle { value: s.lyrics_enabled, onchange: move |v| upd(st, |s| s.lyrics_enabled = v) } }
            Row { label: "Открывать панель, когда текст найден", Toggle { value: s.lyrics_auto_open, onchange: move |v| upd(st, |s| s.lyrics_auto_open = v) } }
            Row { label: "Сторона панели",
                Seg { value: format!("{side:?}"), options: opts(&[("Left", "Слева"), ("Right", "Справа")]),
                    onchange: move |v: String| { let mut t = st.theme; t.write().layout.lyrics_side = if v == "Left" { Side::Left } else { Side::Right }; } }
            }
            Row { label: "Ширина панели", Slider { value: s.lyrics_width, min: 260.0, max: 760.0, step: 10.0, unit: " px", oninput: move |v: f32| upd(st, |s| s.lyrics_width = v) } }
            Row { label: "Размер шрифта", Slider { value: s.lyrics_font, min: 14.0, max: 44.0, step: 1.0, unit: " px", oninput: move |v: f32| upd(st, |s| s.lyrics_font = v) } }
            Row { label: "Выравнивание",
                Seg { value: format!("{:?}", s.lyrics_align), options: opts(&[("Left", "Слева"), ("Center", "По центру"), ("Right", "Справа")]),
                    onchange: move |v: String| upd(st, |s| s.lyrics_align = match v.as_str() { "Left" => LyricsAlign::Left, "Right" => LyricsAlign::Right, _ => LyricsAlign::Center }) }
            }
            Row { label: "Караоке-заливка строки", Toggle { value: s.lyrics_karaoke, onchange: move |v| upd(st, |s| s.lyrics_karaoke = v) } }
            Row { label: "Размывать неактивные строки", Toggle { value: s.lyrics_blur, onchange: move |v| upd(st, |s| s.lyrics_blur = v) } }
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
        Card { title: "Окно", icon: "layout",
            Row { label: "Прозрачность окна", hint: "Сквозь окно виден рабочий стол",
                Slider { value: win_alpha * 100.0, min: 20.0, max: 100.0, step: 1.0, unit: "%", oninput: move |v: f32| { let mut t = st.theme; t.write().glass.window_opacity = v / 100.0; } }
            }
            Row { label: "Эффект окна Windows", hint: "Mica и Acrylic работают в Windows 11; Acrylic может притормаживать при перетаскивании",
                Seg { value: format!("{:?}", s.window_effect), options: opts(&[("None", "Нет"), ("Mica", "Mica"), ("Acrylic", "Acrylic"), ("Blur", "Blur")]),
                    onchange: move |v: String| upd(st, |s| s.window_effect = match v.as_str() { "Mica" => WindowEffect::Mica, "Acrylic" => WindowEffect::Acrylic, "Blur" => WindowEffect::Blur, _ => WindowEffect::None }) }
            }
            Row { label: "Масштаб интерфейса", Slider { value: s.ui_scale * 100.0, min: 70.0, max: 160.0, step: 5.0, unit: "%", oninput: move |v: f32| upd(st, |s| s.ui_scale = v / 100.0) } }
            Row { label: "Поверх всех окон", Toggle { value: s.always_on_top, onchange: move |v| upd(st, |s| s.always_on_top = v) } }
        }
        Card { title: "Анимации и производительность", icon: "sparkles",
            Row { label: "Анимации", Toggle { value: s.animations, onchange: move |v| upd(st, |s| s.animations = v) } }
            Row { label: "Скорость анимаций", hint: "Меньше — быстрее",
                Slider { value: s.anim_speed, min: 0.25, max: 2.5, step: 0.05, decimals: 2, unit: "×", oninput: move |v: f32| upd(st, |s| s.anim_speed = v) }
            }
            Row { label: "Анимация открытия и закрытия окна", Toggle { value: s.open_close_anim, onchange: move |v| upd(st, |s| s.open_close_anim = v) } }
            Row { label: "Заставка при запуске", hint: "Логотип, затем интерфейс плавно собирается по частям. Пропустить — клик или любая клавиша",
                Toggle { value: s.intro_anim, onchange: move |v| upd(st, |s| s.intro_anim = v) }
            }
            Row { label: "Пауза фона, когда окно неактивно", hint: "Звёзды и видео замирают — ноль нагрузки в фоне", Toggle { value: s.pause_bg_unfocused, onchange: move |v| upd(st, |s| s.pause_bg_unfocused = v) } }
            Row { label: "Экономный режим", hint: "Меньше размытия, звёзд и эффектов — для слабых ПК и ноутбуков", Toggle { value: s.low_power, onchange: move |v| upd(st, |s| s.low_power = v) } }
            Row { label: "Цвет акцента из обложки", Toggle { value: s.dynamic_accent, onchange: move |v| {
                upd(st, |s| s.dynamic_accent = v);
                if let Some(t) = st.now.peek().clone() { load_accent(st, &t); }
            } } }
            Row { label: "Пульсация обложки в такт басу", Toggle { value: s.beat_glow, onchange: move |v| upd(st, |s| s.beat_glow = v) } }
        }
        Card { title: "Discord Rich Presence", icon: "cloud",
            p { class: "set-text", "Показывает текущий трек, исполнителя и состояние плеера в Discord. Нужен Application ID приложения Discord Developer Portal; если поле пустое, RPC выключен." }
            Row { label: "Включить Discord RPC", Toggle { value: s.discord_rpc_enabled, onchange: move |v| upd(st, |s| s.discord_rpc_enabled = v) } }
            Row { label: "Application ID", TextInput { value: s.discord_client_id.clone(), placeholder: "например, 123456789012345678", onchange: move |v| upd(st, |s| s.discord_client_id = v) } }
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
        Card { title: "Аккаунт SoundCloud", icon: "cloud",
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
        Card { title: "Как это работает", icon: "info",
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
        Card { title: "Кэш", icon: "storage",
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
        Card { title: "Данные", icon: "folder",
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
        Card { title: "Горячие клавиши", icon: "keyboard",
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
        Card { title: "Nebula", icon: "info",
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
