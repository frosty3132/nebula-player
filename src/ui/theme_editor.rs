//! Редактор тем: галерея, цвета, форма, стекло, шрифт, фон (картинка/GIF/видео),
//! звёзды, раскладка интерфейса (положение панелей, порядок кнопок и разделов),
//! свой CSS и инспектор отдельных элементов (режим редактирования).

use super::actions::*;
use super::icons::Icon;
use super::state::*;
use super::widgets::*;
use crate::theme::*;
use dioxus::prelude::*;

fn opts(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
}

/// Изменение текущей темы (сохранение — автоматически, с задержкой).
pub fn edit(st: AppState, f: impl FnOnce(&mut Theme)) {
    let mut t = st.theme;
    f(&mut t.write());
}

fn apply_theme(st: AppState, t: Theme) {
    let mut s = st.settings;
    s.write().theme = t.id.clone();
    let mut th = st.theme;
    th.set(t);
}

const FONTS: &[(&str, &str)] = &[
    ("\"Segoe UI Variable Text\", \"Segoe UI\", Inter, system-ui, sans-serif", "Segoe UI Variable"),
    ("Inter, \"Segoe UI\", system-ui, sans-serif", "Inter"),
    ("Bahnschrift, \"Segoe UI\", sans-serif", "Bahnschrift"),
    ("\"Trebuchet MS\", sans-serif", "Trebuchet"),
    ("Verdana, sans-serif", "Verdana"),
    ("Georgia, serif", "Georgia"),
    ("\"Cascadia Code\", Consolas, monospace", "Cascadia Code"),
    ("Consolas, monospace", "Consolas"),
];

#[component]
pub fn ThemesTab() -> Element {
    let st = app_state();
    let themes = st.themes.read().clone();
    let cur = st.theme.read().clone();
    let builtin = is_builtin(&cur.id);

    let duplicate = move |_| {
        let mut t = st.theme.peek().clone();
        t.name = format!("{} (копия)", t.name);
        t.id = new_id(&t.name);
        save(&t);
        let mut list = st.themes;
        list.set(load_all());
        apply_theme(st, t);
        toast(st, ToastKind::Success, "Тема скопирована — редактируйте свободно");
    };
    let export_theme = move |_| {
        let t = st.theme.peek().clone();
        spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new().set_file_name(format!("{}.nebula-theme.json", t.id)).add_filter("Тема Nebula", &["json"]).save_file().await {
                match export(&t, f.path()) {
                    Ok(()) => toast(st, ToastKind::Success, "Тема экспортирована"),
                    Err(e) => toast(st, ToastKind::Error, e),
                }
            }
        });
    };
    let import_theme = move |_| {
        spawn(async move {
            if let Some(f) = rfd::AsyncFileDialog::new().add_filter("Тема Nebula", &["json"]).pick_file().await {
                match import(f.path()) {
                    Ok(t) => {
                        let mut list = st.themes;
                        list.set(load_all());
                        apply_theme(st, t);
                        toast(st, ToastKind::Success, "Тема импортирована");
                    }
                    Err(e) => toast(st, ToastKind::Error, e),
                }
            }
        });
    };
    let cid = cur.id.clone();
    rsx! {
        Card { title: "Галерея тем", icon: "palette",
            div { class: "theme-grid",
                for t in themes {
                    {
                        let active = t.id == cur.id;
                        let t2 = t.clone();
                        let style = format!(
                            "--t-bg: {}; --t-surface: {}; --t-accent: {}; --t-accent2: {}; --t-text: {}; --t-r: {}px;",
                            t.colors.bg, t.colors.surface, t.colors.accent, t.colors.accent2, t.colors.text, t.shape.radius.min(22.0)
                        );
                        rsx! {
                            button { key: "{t.id}", class: if active { "theme-card active" } else { "theme-card" }, style: "{style}",
                                onclick: move |_| apply_theme(st, t2.clone()),
                                div { class: "tc-preview",
                                    span { class: "tp-side" }
                                    span { class: "tp-main", span { class: "tp-line" } span { class: "tp-line short" } }
                                    span { class: "tp-player", span { class: "tp-dot" } }
                                }
                                div { class: "tc-name", "{t.name}" }
                            }
                        }
                    }
                }
            }
            div { class: "set-actions",
                button { class: "btn", onclick: duplicate, Icon { name: "copy", size: 18 } "Дублировать" }
                button { class: "btn ghost", onclick: import_theme, Icon { name: "upload", size: 18 } "Импорт" }
                button { class: "btn ghost", onclick: export_theme, Icon { name: "download", size: 18 } "Экспорт" }
                button { class: "btn danger",
                    onclick: move |_| {
                        let mut m = st.modal;
                        m.set(Some(Modal::Confirm {
                            title: if builtin { "Сбросить тему?".into() } else { "Удалить тему?".into() },
                            text: if builtin { "Все ваши правки этой встроенной темы будут отменены.".into() } else { "Пользовательская тема будет удалена.".into() },
                            action: ConfirmAction::DeleteTheme(cid.clone()),
                        }));
                    },
                    Icon { name: if builtin { "refresh" } else { "delete" }, size: 18 }
                    if builtin { "Сбросить к исходной" } else { "Удалить" }
                }
            }
            Row { label: "Название темы",
                TextInput { value: cur.name.clone(), onchange: move |v: String| edit(st, |t| t.name = v) }
            }
            Row { label: "Тёмная тема", hint: "Влияет на системные элементы (полосы прокрутки, поля ввода)",
                Toggle { value: cur.dark, onchange: move |v| edit(st, |t| t.dark = v) }
            }
        }
        ColorsCard {}
        ShapeCard {}
        BackgroundCard {}
        Card { title: "Свой CSS", icon: "code",
            p { class: "set-text", "Полный контроль: селекторы [data-el=\"player\"], .track-row, CSS-переменные --accent, --radius и т. д. Применяется сразу." }
            textarea {
                class: "code-area",
                spellcheck: false,
                value: "{cur.custom_css}",
                placeholder: "[data-el=\"player\"] {{ border-radius: 0; }}\n.track-row:hover {{ transform: translateX(4px); }}",
                oninput: move |e: FormEvent| edit(st, |t| t.custom_css = e.value()),
            }
        }
    }
}

#[component]
fn ColorsCard() -> Element {
    let st = app_state();
    let c = st.theme.read().colors.clone();
    let fields: [(&str, &str, String); 10] = [
        ("bg", "Фон", c.bg.clone()),
        ("surface", "Панели", c.surface.clone()),
        ("surface2", "Панели (второй тон)", c.surface2.clone()),
        ("text", "Текст", c.text.clone()),
        ("muted", "Второстепенный текст", c.muted.clone()),
        ("accent", "Акцент", c.accent.clone()),
        ("accent2", "Второй акцент", c.accent2.clone()),
        ("glow", "Свечение", c.glow.clone()),
        ("danger", "Ошибки", c.danger.clone()),
        ("border", "Рамки", c.border.clone()),
    ];
    rsx! {
        Card { title: "Цвета", icon: "palette",
            div { class: "color-grid",
                for (key , label , value) in fields {
                    div { key: "{key}", class: "color-cell",
                        span { class: "set-title", "{label}" }
                        ColorInput { value, onchange: move |v: String| edit(st, |t| {
                            let c = &mut t.colors;
                            match key {
                                "bg" => c.bg = v,
                                "surface" => c.surface = v,
                                "surface2" => c.surface2 = v,
                                "text" => c.text = v,
                                "muted" => c.muted = v,
                                "accent" => c.accent = v,
                                "accent2" => c.accent2 = v,
                                "glow" => c.glow = v,
                                "danger" => c.danger = v,
                                _ => c.border = v,
                            }
                        }) }
                    }
                }
            }
        }
    }
}

#[component]
fn ShapeCard() -> Element {
    let st = app_state();
    let t = st.theme.read().clone();
    let (sh, gl, f) = (t.shape, t.glass, t.font);
    let font_opts: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = FONTS.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        if !v.iter().any(|(a, _)| *a == f.family) {
            v.insert(0, (f.family.clone(), "Свой шрифт".into()));
        }
        v
    };
    rsx! {
        Card { title: "Форма и стекло", icon: "layout",
            Row { label: "Скругление панелей", Slider { value: sh.radius, min: 0.0, max: 40.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.shape.radius = v) } }
            Row { label: "Скругление мелких элементов", Slider { value: sh.radius_small, min: 0.0, max: 30.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.shape.radius_small = v) } }
            Row { label: "Скругление крупных блоков", Slider { value: sh.radius_large, min: 0.0, max: 60.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.shape.radius_large = v) } }
            Row { label: "Скругление обложек", Slider { value: sh.cover_radius, min: 0.0, max: 60.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.shape.cover_radius = v) } }
            Row { label: "Скругление кнопок", hint: "999 — капсулы",
                Slider { value: sh.button_radius.min(40.0), min: 0.0, max: 40.0, step: 1.0, unit: " px", oninput: move |v: f32| edit(st, |t| t.shape.button_radius = if v >= 40.0 { 999.0 } else { v }) }
            }
            Row { label: "Толщина рамок", Slider { value: sh.border_width, min: 0.0, max: 4.0, step: 0.5, decimals: 1, unit: " px", oninput: move |v| edit(st, |t| t.shape.border_width = v) } }
            Row { label: "Жидкое стекло (как в iOS)", hint: "Молочные панели со световым краем и глубокой тенью",
                Toggle { value: gl.liquid, onchange: move |v| edit(st, |t| t.glass.liquid = v) }
            }
            Row { label: "Размытие стекла", Slider { value: gl.blur, min: 0.0, max: 60.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.glass.blur = v) } }
            Row { label: "Плотность стекла", Slider { value: gl.opacity * 100.0, min: 0.0, max: 100.0, step: 1.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.glass.opacity = v / 100.0) } }
            Row { label: "Насыщенность за стеклом", Slider { value: gl.saturation, min: 50.0, max: 250.0, step: 5.0, unit: "%", oninput: move |v| edit(st, |t| t.glass.saturation = v) } }
            Row { label: "Сила свечения", Slider { value: gl.glow * 100.0, min: 0.0, max: 100.0, step: 1.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.glass.glow = v / 100.0) } }
            Row { label: "Прозрачность окна", Slider { value: gl.window_opacity * 100.0, min: 20.0, max: 100.0, step: 1.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.glass.window_opacity = v / 100.0) } }
            Row { label: "Шрифт", Select { value: f.family.clone(), options: font_opts, onchange: move |v: String| edit(st, |t| t.font.family = v) } }
            Row { label: "Свой шрифт (CSS font-family)", TextInput { value: f.family.clone(), onchange: move |v: String| edit(st, |t| t.font.family = v) } }
            Row { label: "Размер текста", Slider { value: f.size, min: 11.0, max: 20.0, step: 0.5, decimals: 1, unit: " px", oninput: move |v| edit(st, |t| t.font.size = v) } }
            Row { label: "Жирность заголовков", Slider { value: f.title_weight as f32, min: 300.0, max: 900.0, step: 100.0, oninput: move |v: f32| edit(st, |t| t.font.title_weight = v as u16) } }
            Row { label: "Межбуквенный интервал", Slider { value: f.letter_spacing, min: -1.0, max: 3.0, step: 0.1, decimals: 1, unit: " px", oninput: move |v| edit(st, |t| t.font.letter_spacing = v) } }
        }
    }
}

#[component]
fn BackgroundCard() -> Element {
    let st = app_state();
    let t = st.theme.read().clone();
    let (bg, stars) = (t.background, t.stars);
    let kind = format!("{:?}", bg.kind);
    let media = matches!(bg.kind, BgKind::Image | BgKind::Video);
    let pick = move |_| {
        let video = st.theme.peek().background.kind == BgKind::Video;
        spawn(async move {
            let dlg = if video {
                rfd::AsyncFileDialog::new().add_filter("Видео", &["mp4", "webm", "mov", "m4v"])
            } else {
                rfd::AsyncFileDialog::new().add_filter("Изображения и GIF", &["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp"])
            };
            if let Some(f) = dlg.set_title("Фон для Nebula").pick_file().await {
                let p = f.path().display().to_string();
                edit(st, |t| t.background.source = p);
            }
        });
    };
    rsx! {
        Card { title: "Фон", icon: "image",
            Row { label: "Тип фона",
                Seg { value: kind, options: opts(&[("Space", "Космос"), ("Gradient", "Градиент"), ("Solid", "Цвет"), ("Image", "Картинка/GIF"), ("Video", "Видео"), ("Cover", "Обложка")]),
                    onchange: move |v: String| edit(st, |t| t.background.kind = match v.as_str() {
                        "Gradient" => BgKind::Gradient, "Solid" => BgKind::Solid, "Image" => BgKind::Image, "Video" => BgKind::Video, "Cover" => BgKind::Cover, _ => BgKind::Space,
                    }) }
            }
            if media {
                Row { label: "Файл или URL", hint: "Для анимации лучше видео (mp4/webm): оно декодируется видеокартой и почти не нагружает CPU, в отличие от GIF",
                    div { class: "row-inline",
                        TextInput { value: bg.source.clone(), placeholder: "C:\\Wallpapers\\space.gif или https://…", onchange: move |v: String| edit(st, |t| t.background.source = v) }
                        button { class: "btn small", onclick: pick, Icon { name: "folder", size: 16 } "Выбрать" }
                    }
                }
                Row { label: "Заполнение",
                    Seg { value: format!("{:?}", bg.fit), options: opts(&[("Cover", "Заполнить"), ("Contain", "Вписать"), ("Tile", "Плитка"), ("Stretch", "Растянуть")]),
                        onchange: move |v: String| edit(st, |t| t.background.fit = match v.as_str() { "Contain" => BgFit::Contain, "Tile" => BgFit::Tile, "Stretch" => BgFit::Stretch, _ => BgFit::Cover }) }
                }
                Row { label: "Звёзды поверх картинки", Toggle { value: bg.stars_over_media, onchange: move |v| edit(st, |t| t.background.stars_over_media = v) } }
            }
            Row { label: "Размытие фона", Slider { value: bg.blur, min: 0.0, max: 60.0, step: 1.0, unit: " px", oninput: move |v| edit(st, |t| t.background.blur = v) } }
            Row { label: "Затемнение", Slider { value: bg.dim * 100.0, min: 0.0, max: 95.0, step: 1.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.background.dim = v / 100.0) } }
            Row { label: "Туманности", Toggle { value: bg.nebula, onchange: move |v| edit(st, |t| t.background.nebula = v) } }
            Row { label: "Яркость туманностей", Slider { value: bg.nebula_intensity * 100.0, min: 0.0, max: 150.0, step: 5.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.background.nebula_intensity = v / 100.0) } }
        }
        Card { title: "Звёзды", icon: "sparkles",
            Row { label: "Плотность", Slider { value: stars.density * 100.0, min: 0.0, max: 250.0, step: 5.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.stars.density = v / 100.0) } }
            Row { label: "Скорость полёта", Slider { value: stars.speed * 100.0, min: 0.0, max: 400.0, step: 5.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.stars.speed = v / 100.0) } }
            Row { label: "Размер", Slider { value: stars.size * 100.0, min: 50.0, max: 300.0, step: 10.0, unit: "%", oninput: move |v: f32| edit(st, |t| t.stars.size = v / 100.0) } }
            Row { label: "Цвет звёзд", ColorInput { value: stars.color.clone(), onchange: move |v: String| edit(st, |t| t.stars.color = v) } }
            Row { label: "Мерцание", Toggle { value: stars.twinkle, onchange: move |v| edit(st, |t| t.stars.twinkle = v) } }
            Row { label: "Падающие звёзды", Toggle { value: stars.shooting, onchange: move |v| edit(st, |t| t.stars.shooting = v) } }
        }
    }
}

// ───────────────────────────────── расположение ─────────────────────────────────

#[component]
fn OrderList(items: Vec<(String, String, bool)>, lock: Option<String>, on_move: EventHandler<(usize, usize)>, on_toggle: EventHandler<usize>) -> Element {
    let n = items.len();
    rsx! {
        div { class: "order-list",
            for (i , (id , label , visible)) in items.into_iter().enumerate() {
                div { key: "{id}", class: if visible { "order-item" } else { "order-item off" },
                    Icon { name: "drag", size: 16 }
                    span { class: "flex1", "{label}" }
                    IconBtn { icon: "up", size: 15, title: "Выше", class: if i == 0 { "dim" } else { "" }, onclick: move |_| if i > 0 { on_move.call((i, i - 1)) } }
                    IconBtn { icon: "down", size: 15, title: "Ниже", class: if i + 1 == n { "dim" } else { "" }, onclick: move |_| if i + 1 < n { on_move.call((i, i + 1)) } }
                    if lock.as_deref() != Some(id.as_str()) {
                        IconBtn { icon: if visible { "eye" } else { "eye_off" }, size: 15, title: if visible { "Скрыть" } else { "Показать" }, onclick: move |_| on_toggle.call(i) }
                    } else {
                        span { class: "order-lock", title: "Этот элемент нельзя скрыть", "🔒" }
                    }
                }
            }
        }
    }
}

fn swap<T>(v: &mut [T], (a, b): (usize, usize)) {
    if a < v.len() && b < v.len() {
        v.swap(a, b);
    }
}

#[component]
pub fn LayoutTab() -> Element {
    let st = app_state();
    let l = st.theme.read().layout.clone();

    let controls: Vec<(String, String, bool)> = Control::ALL
        .iter()
        .map(|c| (format!("{c:?}"), c.label().to_string(), l.controls.contains(c)))
        .collect::<Vec<_>>();
    // порядок: видимые — как в теме, скрытые — в конце
    let mut ctrl_sorted: Vec<(String, String, bool)> = l.controls.iter().map(|c| (format!("{c:?}"), c.label().to_string(), true)).collect();
    for c in controls.into_iter().filter(|c| !c.2) {
        ctrl_sorted.push(c);
    }
    let mut extras_sorted: Vec<(String, String, bool)> = l.extras.iter().map(|c| (format!("{c:?}"), c.label().to_string(), true)).collect();
    for x in Extra::ALL.iter().filter(|x| !l.extras.contains(x)) {
        extras_sorted.push((format!("{x:?}"), x.label().to_string(), false));
    }
    let nav: Vec<(String, String, bool)> = l
        .nav
        .iter()
        .map(|e| (e.id.clone(), NAV_ITEMS.iter().find(|n| n.0 == e.id).map(|n| n.1).unwrap_or("?").to_string(), e.visible))
        .collect();
    let home: Vec<(String, String, bool)> = l
        .home
        .iter()
        .map(|e| (e.id.clone(), HOME_SECTIONS.iter().find(|n| n.0 == e.id).map(|n| n.1).unwrap_or("?").to_string(), e.visible))
        .collect();

    let ctrl_ids: Vec<String> = ctrl_sorted.iter().map(|c| c.0.clone()).collect();
    let ctrl_ids2 = ctrl_ids.clone();
    let extra_ids: Vec<String> = extras_sorted.iter().map(|c| c.0.clone()).collect();
    let extra_ids2 = extra_ids.clone();

    rsx! {
        Card { title: "Панели", icon: "layout",
            Row { label: "Меню разделов", hint: "Сверху — панель-док со значками, которые увеличиваются при наведении",
                Seg {
                    value: if l.sidebar == SidebarPos::Hidden { "Top".to_string() } else { format!("{:?}", l.sidebar) },
                    options: opts(&[("Top", "Сверху (значки)"), ("Left", "Слева"), ("Right", "Справа")]),
                    onchange: move |v: String| edit(st, |t| t.layout.sidebar = match v.as_str() { "Left" => SidebarPos::Left, "Right" => SidebarPos::Right, _ => SidebarPos::Top }),
                }
            }
            Row { label: "Компактное меню (только иконки)", Toggle { value: l.sidebar_compact, onchange: move |v| edit(st, |t| t.layout.sidebar_compact = v) } }
            Row { label: "Ширина меню", Slider { value: l.sidebar_width, min: 180.0, max: 360.0, step: 4.0, unit: " px", oninput: move |v| edit(st, |t| t.layout.sidebar_width = v) } }
            Row { label: "Логотип в меню", Toggle { value: l.show_logo, onchange: move |v| edit(st, |t| t.layout.show_logo = v) } }
            Row { label: "Название в заголовке окна", Toggle { value: l.titlebar_title, onchange: move |v| edit(st, |t| t.layout.titlebar_title = v) } }
            Row { label: "Плеер",
                Seg { value: format!("{:?}", l.player), options: opts(&[("Bottom", "Снизу"), ("Top", "Сверху"), ("Floating", "Парящий")]),
                    onchange: move |v: String| edit(st, |t| t.layout.player = match v.as_str() { "Top" => PlayerPos::Top, "Floating" => PlayerPos::Floating, _ => PlayerPos::Bottom }) }
            }
            Row { label: "Вид плеера",
                Seg { value: format!("{:?}", l.player_style), options: opts(&[("Classic", "Классика"), ("Centered", "По центру"), ("Minimal", "Минимум")]),
                    onchange: move |v: String| edit(st, |t| t.layout.player_style = match v.as_str() { "Centered" => PlayerStyle::Centered, "Minimal" => PlayerStyle::Minimal, _ => PlayerStyle::Classic }) }
            }
            Row { label: "Дизайн плеера Material 3", hint: "Волнистая полоса прогресса, кнопка-«сквиркл», тональные кнопки — как MD3-плеер",
                Toggle { value: l.player_md3, onchange: move |v| edit(st, |t| t.layout.player_md3 = v) }
            }
            Row { label: "Высота плеера", Slider { value: l.player_height, min: 64.0, max: 140.0, step: 2.0, unit: " px", oninput: move |v| edit(st, |t| t.layout.player_height = v) } }
            Row { label: "Панель текста",
                Seg { value: format!("{:?}", l.lyrics_side), options: opts(&[("Left", "Слева"), ("Right", "Справа")]),
                    onchange: move |v: String| edit(st, |t| t.layout.lyrics_side = if v == "Left" { Side::Left } else { Side::Right }) }
            }
        }
        Card { title: "Контент", icon: "grid",
            Row { label: "Форма обложки в плеере",
                Seg { value: format!("{:?}", l.cover_shape), options: opts(&[("Square", "Квадрат"), ("Rounded", "Скруглённая"), ("Circle", "Круг"), ("Vinyl", "Винил")]),
                    onchange: move |v: String| edit(st, |t| t.layout.cover_shape = match v.as_str() { "Square" => CoverShape::Square, "Circle" => CoverShape::Circle, "Vinyl" => CoverShape::Vinyl, _ => CoverShape::Rounded }) }
            }
            Row { label: "Плотность списков",
                Seg { value: format!("{:?}", l.density), options: opts(&[("Comfortable", "Просторно"), ("Compact", "Компактно")]),
                    onchange: move |v: String| edit(st, |t| t.layout.density = if v == "Compact" { Density::Compact } else { Density::Comfortable }) }
            }
            Row { label: "Размер карточек", Slider { value: l.card_size, min: 120.0, max: 260.0, step: 4.0, unit: " px", oninput: move |v| edit(st, |t| t.layout.card_size = v) } }
            Row { label: "Отступы страницы", Slider { value: l.content_padding, min: 8.0, max: 64.0, step: 2.0, unit: " px", oninput: move |v| edit(st, |t| t.layout.content_padding = v) } }
        }
        Card { title: "Кнопки управления", icon: "play",
            p { class: "set-text", "Порядок и видимость кнопок плеера. Play скрыть нельзя." }
            OrderList { items: ctrl_sorted, lock: Some("Play".to_string()),
                on_move: move |(a, b): (usize, usize)| {
                    let mut ids = ctrl_ids.clone();
                    swap(&mut ids, (a, b));
                    edit(st, |t| {
                        let visible: Vec<Control> = ids.iter().filter_map(|id| Control::ALL.iter().find(|c| format!("{c:?}") == *id).copied()).filter(|c| t.layout.controls.contains(c)).collect();
                        t.layout.controls = visible;
                    });
                },
                on_toggle: move |i: usize| {
                    let id = ctrl_ids2[i].clone();
                    edit(st, |t| {
                        if let Some(c) = Control::ALL.iter().find(|c| format!("{c:?}") == id).copied() {
                            if t.layout.controls.contains(&c) { t.layout.controls.retain(|x| *x != c) } else { t.layout.controls.push(c) }
                        }
                    });
                },
            }
        }
        Card { title: "Дополнительные кнопки плеера", icon: "tune",
            OrderList { items: extras_sorted, lock: None,
                on_move: move |(a, b): (usize, usize)| {
                    let mut ids = extra_ids.clone();
                    swap(&mut ids, (a, b));
                    edit(st, |t| {
                        let visible: Vec<Extra> = ids.iter().filter_map(|id| Extra::ALL.iter().find(|c| format!("{c:?}") == *id).copied()).filter(|c| t.layout.extras.contains(c)).collect();
                        t.layout.extras = visible;
                    });
                },
                on_toggle: move |i: usize| {
                    let id = extra_ids2[i].clone();
                    edit(st, |t| {
                        if let Some(c) = Extra::ALL.iter().find(|c| format!("{c:?}") == id).copied() {
                            if t.layout.extras.contains(&c) { t.layout.extras.retain(|x| *x != c) } else { t.layout.extras.push(c) }
                        }
                    });
                },
            }
        }
        Card { title: "Пункты меню", icon: "queue",
            OrderList { items: nav, lock: Some("home".to_string()),
                on_move: move |ab| edit(st, |t| swap(&mut t.layout.nav, ab)),
                on_toggle: move |i: usize| edit(st, |t| if let Some(e) = t.layout.nav.get_mut(i) { e.visible = !e.visible }),
            }
        }
        Card { title: "Разделы главной", icon: "home",
            OrderList { items: home, lock: None,
                on_move: move |ab| edit(st, |t| swap(&mut t.layout.home, ab)),
                on_toggle: move |i: usize| edit(st, |t| if let Some(e) = t.layout.home.get_mut(i) { e.visible = !e.visible }),
            }
        }
    }
}

// ───────────────────────────────── элементы ─────────────────────────────────

#[component]
pub fn ElementsTab() -> Element {
    let st = app_state();
    let styled: Vec<String> = st.theme.read().elements.keys().cloned().collect();
    let mut groups: Vec<&str> = Vec::new();
    for e in ELEMENTS {
        if !groups.contains(&e.group) {
            groups.push(e.group);
        }
    }
    rsx! {
        Card { title: "Каждый элемент — отдельно", icon: "brush",
            p { class: "set-text", "Включите режим редактирования (кисть в заголовке окна или Ctrl+E) и кликните по любому элементу интерфейса — откроется инспектор: скругление, фон, прозрачность, размытие, отступы, шрифт, рамка, свечение, масштаб, смещение, порядок и свой CSS. Или выберите элемент из списка." }
            div { class: "set-actions",
                button { class: "btn primary",
                    onclick: move |_| { let mut e = st.edit_mode; e.set(true); },
                    Icon { name: "brush", size: 18 } "Включить режим редактирования"
                }
                button { class: "btn ghost",
                    onclick: move |_| { edit(st, |t| t.elements.clear()); toast(st, ToastKind::Info, "Стили всех элементов сброшены"); },
                    Icon { name: "refresh", size: 18 } "Сбросить все элементы"
                }
            }
        }
        for g in groups {
            Card { key: "{g}", title: "{g}",
                div { class: "el-list",
                    for e in ELEMENTS.iter().filter(|e| e.group == g) {
                        button {
                            key: "{e.id}",
                            class: if styled.iter().any(|s| s == e.id) { "el-item styled" } else { "el-item" },
                            onclick: move |_| {
                                let mut sel = st.selected_el;
                                sel.set(Some(e.id.to_string()));
                                let mut em = st.edit_mode;
                                em.set(true);
                            },
                            span { class: "el-name", "{e.name}" }
                            code { "{e.id}" }
                            if styled.iter().any(|s| s == e.id) {
                                span { class: "badge", "изменён" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn opt_slider(
    st: AppState,
    id: String,
    label: &'static str,
    value: Option<f32>,
    default: f32,
    min: f32,
    max: f32,
    step: f32,
    unit: &'static str,
    set: fn(&mut ElementStyle, Option<f32>),
) -> Element {
    let id2 = id.clone();
    rsx! {
        div { class: "ins-row",
            div { class: "ins-label",
                span { "{label}" }
                if value.is_some() {
                    button { class: "mini", title: "Сбросить", onclick: move |_| edit(st, |t| { let e = t.elements.entry(id2.clone()).or_default(); set(e, None); }), "↺" }
                }
            }
            Slider { value: value.unwrap_or(default), min, max, step, unit: unit.to_string(), decimals: if step < 1.0 { 2 } else { 0 },
                oninput: move |v: f32| edit(st, |t| { let e = t.elements.entry(id.clone()).or_default(); set(e, Some(v)); }) }
        }
    }
}

/// Плавающий инспектор выбранного элемента (режим редактирования).
#[component]
pub fn Inspector() -> Element {
    let st = app_state();
    let Some(id) = st.selected_el.read().clone() else { return rsx! {} };
    let s = st.theme.read().style_of(&id);
    let name = element_name(&id);
    let close = move |_| {
        let mut sel = st.selected_el;
        sel.set(None);
    };
    let (i1, i2, i3, i4, i5, i6, i7, i8) = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    rsx! {
        div { class: "inspector glass", onclick: move |e| e.stop_propagation(),
            div { class: "ins-head",
                div {
                    div { class: "ins-title", "{name}" }
                    code { class: "ins-id", "[data-el=\"{id}\"]" }
                }
                div { class: "flex1" }
                IconBtn { icon: "refresh", size: 17, title: "Сбросить элемент", onclick: move |_| edit(st, |t| { t.elements.remove(&i1); }) }
                IconBtn { icon: "close", size: 17, title: "Закрыть", onclick: close }
            }
            div { class: "ins-body",
                if crate::theme::is_essential(&id) {
                    div { class: "ins-row inline",
                        span { "Скрыть элемент" }
                        span { class: "order-lock", title: "Без этого элемента окно будет пустым", "🔒 нельзя" }
                    }
                } else {
                    div { class: "ins-row inline",
                        span { "Скрыть элемент" }
                        Toggle { value: s.hidden, onchange: move |v| edit(st, |t| t.elements.entry(i2.clone()).or_default().hidden = v) }
                    }
                }
                {opt_slider(st, id.clone(), "Скругление", s.radius, 12.0, 0.0, 80.0, 1.0, " px", |e, v| e.radius = v)}
                {opt_slider(st, id.clone(), "Прозрачность", s.opacity, 1.0, 0.0, 1.0, 0.01, "", |e, v| e.opacity = v)}
                {opt_slider(st, id.clone(), "Размытие фона", s.blur, 20.0, 0.0, 60.0, 1.0, " px", |e, v| e.blur = v)}
                {opt_slider(st, id.clone(), "Внутренний отступ", s.padding, 8.0, 0.0, 60.0, 1.0, " px", |e, v| e.padding = v)}
                {opt_slider(st, id.clone(), "Внешний отступ", s.margin, 0.0, 0.0, 60.0, 1.0, " px", |e, v| e.margin = v)}
                {opt_slider(st, id.clone(), "Размер шрифта", s.font_size, 14.0, 8.0, 48.0, 1.0, " px", |e, v| e.font_size = v)}
                {opt_slider(st, id.clone(), "Толщина рамки", s.border_width, 1.0, 0.0, 8.0, 0.5, " px", |e, v| e.border_width = v)}
                {opt_slider(st, id.clone(), "Свечение", s.glow, 0.5, 0.0, 1.5, 0.05, "", |e, v| e.glow = v)}
                {opt_slider(st, id.clone(), "Масштаб", s.scale, 1.0, 0.5, 1.6, 0.01, "×", |e, v| e.scale = v)}
                {opt_slider(st, id.clone(), "Смещение X", s.offset_x, 0.0, -400.0, 400.0, 1.0, " px", |e, v| e.offset_x = v)}
                {opt_slider(st, id.clone(), "Смещение Y", s.offset_y, 0.0, -300.0, 300.0, 1.0, " px", |e, v| e.offset_y = v)}
                {opt_slider(st, id.clone(), "Ширина", s.width, 240.0, 20.0, 1200.0, 2.0, " px", |e, v| e.width = v)}
                {opt_slider(st, id.clone(), "Высота", s.height, 60.0, 10.0, 900.0, 2.0, " px", |e, v| e.height = v)}
                div { class: "ins-row",
                    div { class: "ins-label", span { "Порядок в ряду" } }
                    div { class: "row-inline",
                        button { class: "btn small", onclick: move |_| edit(st, |t| { let e = t.elements.entry(i3.clone()).or_default(); e.order = Some(e.order.unwrap_or(0) - 1); }), "← Раньше" }
                        span { class: "mono", "{s.order.unwrap_or(0)}" }
                        button { class: "btn small", onclick: move |_| edit(st, |t| { let e = t.elements.entry(i4.clone()).or_default(); e.order = Some(e.order.unwrap_or(0) + 1); }), "Позже →" }
                    }
                }
                div { class: "ins-row",
                    div { class: "ins-label", span { "Фон" } }
                    ColorInput { value: s.background.clone().unwrap_or_default(), onchange: move |v: String| edit(st, |t| t.elements.entry(i5.clone()).or_default().background = if v.trim().is_empty() { None } else { Some(v) }) }
                }
                div { class: "ins-row",
                    div { class: "ins-label", span { "Цвет текста" } }
                    ColorInput { value: s.color.clone().unwrap_or_default(), onchange: move |v: String| edit(st, |t| t.elements.entry(i6.clone()).or_default().color = if v.trim().is_empty() { None } else { Some(v) }) }
                }
                div { class: "ins-row",
                    div { class: "ins-label", span { "Цвет рамки" } }
                    ColorInput { value: s.border_color.clone().unwrap_or_default(), onchange: move |v: String| edit(st, |t| t.elements.entry(i7.clone()).or_default().border_color = if v.trim().is_empty() { None } else { Some(v) }) }
                }
                div { class: "ins-row",
                    div { class: "ins-label", span { "Свой CSS" } span { class: "set-hint", "свойства или правила, & — сам элемент" } }
                    textarea { class: "code-area small", spellcheck: false, value: "{s.custom_css}", placeholder: "letter-spacing: 2px;\ntext-transform: uppercase;",
                        oninput: move |e: FormEvent| edit(st, |t| t.elements.entry(i8.clone()).or_default().custom_css = e.value()) }
                }
            }
        }
    }
}
