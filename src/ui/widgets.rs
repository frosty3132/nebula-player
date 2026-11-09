//! Переиспользуемые элементы управления: переключатели, слайдеры, списки,
//! выбор цвета, сегментированные кнопки, строки настроек.

use super::icons::Icon;
use dioxus::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

/// Класс анимации для содержимого вкладки: `fwd`, если новая вкладка дальше по
/// списку, `back` — если ближе, `first` — при первом показе. Значение стабильно,
/// пока вкладка не меняется (иначе анимация перезапускалась бы при каждой отрисовке).
/// Хук: вызывать безусловно, до любых ранних `return`.
pub fn use_switch_dir(index: usize, back: &'static str, fwd: &'static str, first: &'static str) -> &'static str {
    let mem = use_hook(|| Rc::new(Cell::new((index, first))));
    let (prev, cls) = mem.get();
    if prev == index {
        return cls;
    }
    let cls = if index > prev { fwd } else { back };
    mem.set((index, cls));
    cls
}

#[component]
pub fn Toggle(value: bool, onchange: EventHandler<bool>) -> Element {
    rsx! {
        button {
            class: if value { "toggle on" } else { "toggle" },
            "data-el": "button",
            role: "switch",
            onclick: move |e| {
                e.stop_propagation();
                onchange.call(!value)
            },
            span { class: "knob" }
        }
    }
}

#[component]
pub fn Slider(
    value: f32,
    min: f32,
    max: f32,
    #[props(default = 1.0)] step: f32,
    #[props(default)] unit: String,
    #[props(default = 0)] decimals: usize,
    #[props(default)] mark: Option<f32>,
    /// Применять значение только при отпускании ползунка (пока тянешь —
    /// меняется лишь подпись). Нужно, когда значение двигает сам ползунок:
    /// масштаб интерфейса растягивает страницу, ползунок уезжает из-под
    /// курсора и значение убегало к максимуму.
    #[props(default)]
    commit_on_release: bool,
    oninput: EventHandler<f32>,
) -> Element {
    let mut draft = use_signal(|| None::<f32>);
    let value = draft.read().unwrap_or(value);
    let pct = if max > min { ((value - min) / (max - min) * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let label = format!("{:.*}{}", decimals, value, unit);
    let mark_style = mark.map(|m| format!("left: {}%;", ((m - min) / (max - min) * 100.0).clamp(0.0, 100.0)));
    rsx! {
        div { class: "slider",
            div { class: "slider-track",
                input {
                    r#type: "range",
                    class: "range",
                    min: "{min}",
                    max: "{max}",
                    step: "{step}",
                    value: "{value}",
                    style: "--p: {pct}%;",
                    oninput: move |e: FormEvent| {
                        if let Ok(v) = e.value().parse::<f32>() {
                            if commit_on_release {
                                draft.set(Some(v));
                            } else {
                                oninput.call(v)
                            }
                        }
                    },
                    onchange: move |e: FormEvent| {
                        if commit_on_release {
                            if let Ok(v) = e.value().parse::<f32>() {
                                oninput.call(v);
                            }
                            draft.set(None);
                        }
                    },
                }
                if let Some(ms) = mark_style {
                    span { class: "slider-mark", style: "{ms}" }
                }
            }
            span { class: "slider-value", "{label}" }
        }
    }
}

/// Выпадающий список со стеклянной панелью. Родной `<select>` рисует сама
/// Windows — его список нельзя ни размыть, ни скруглить, поэтому свой.
/// Закрывается выбором, Esc или щелчком мимо (потеря фокуса кнопкой).
#[component]
pub fn Select(value: String, options: Vec<(String, String)>, onchange: EventHandler<String>) -> Element {
    let mut open = use_signal(|| false);
    let current = options.iter().find(|(v, _)| *v == value).map(|(_, l)| l.clone()).unwrap_or_else(|| value.clone());
    rsx! {
        div {
            class: if open() { "dd open" } else { "dd" },
            "data-el": "input",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Escape && open() {
                    e.stop_propagation();
                    open.set(false);
                }
            },
            button {
                class: "dd-btn",
                r#type: "button",
                "aria-haspopup": "listbox",
                "aria-expanded": "{open()}",
                onclick: move |_| open.toggle(),
                onfocusout: move |_| open.set(false),
                span { class: "dd-label ellipsis", "{current}" }
                span { class: "dd-chevron", Icon { name: "down", size: 16 } }
            }
            if open() {
                div {
                    class: "dd-panel",
                    role: "listbox",
                    // щелчок внутри панели не забирает фокус у кнопки — иначе
                    // список закрылся бы раньше, чем засчитается выбор
                    onmousedown: move |e| e.prevent_default(),
                    for (v , label) in options.iter().cloned() {
                        button {
                            key: "{v}",
                            class: if v == value { "dd-opt active" } else { "dd-opt" },
                            r#type: "button",
                            role: "option",
                            onclick: move |_| {
                                open.set(false);
                                onchange.call(v.clone());
                            },
                            span { class: "ellipsis", "{label}" }
                            if v == value {
                                Icon { name: "check", size: 16 }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn Seg(value: String, options: Vec<(String, String)>, onchange: EventHandler<String>) -> Element {
    rsx! {
        div { class: "seg", "data-el": "chip", "data-glide": ".seg-btn.active",
            span { class: "glider seg-glider" }
            for (v , label) in options.iter().cloned() {
                button {
                    class: if v == value { "seg-btn active" } else { "seg-btn" },
                    onclick: move |_| onchange.call(v.clone()),
                    "{label}"
                }
            }
        }
    }
}

#[component]
pub fn ColorInput(value: String, onchange: EventHandler<String>) -> Element {
    // <input type=color> понимает только #rrggbb; rgba и прочее правится текстом
    let hex = if value.starts_with('#') && (value.len() == 7 || value.len() == 4) { value.clone() } else { "#888888".to_string() };
    rsx! {
        div { class: "color-input",
            input {
                r#type: "color",
                value: "{hex}",
                oninput: move |e: FormEvent| onchange.call(e.value()),
            }
            input {
                class: "input mono",
                "data-el": "input",
                value: "{value}",
                spellcheck: false,
                onchange: move |e: FormEvent| onchange.call(e.value()),
            }
        }
    }
}

#[component]
pub fn TextInput(
    value: String,
    #[props(default)] placeholder: String,
    #[props(default)] password: bool,
    onchange: EventHandler<String>,
) -> Element {
    rsx! {
        input {
            class: "input",
            "data-el": "input",
            r#type: if password { "password" } else { "text" },
            value: "{value}",
            placeholder: "{placeholder}",
            spellcheck: false,
            onchange: move |e: FormEvent| onchange.call(e.value()),
        }
    }
}

#[component]
pub fn Row(label: String, #[props(default)] hint: String, children: Element) -> Element {
    rsx! {
        div { class: "set-row",
            div { class: "set-label",
                div { class: "set-title", "{label}" }
                if !hint.is_empty() {
                    div { class: "set-hint", "{hint}" }
                }
            }
            div { class: "set-control", {children} }
        }
    }
}

/// Карточка настроек: плитка с иконкой, заголовок и короткое описание, под
/// ними строки. `wide` — на всю ширину сетки (например, эквалайзер).
#[component]
pub fn Card(
    title: String,
    #[props(default)] icon: String,
    #[props(default)] desc: String,
    #[props(default)] wide: bool,
    children: Element,
) -> Element {
    rsx! {
        section { class: if wide { "set-card glass wide" } else { "set-card glass" },
            header { class: "set-card-head",
                if !icon.is_empty() {
                    span { class: "set-card-icon", Icon { name: icon.clone(), size: 18 } }
                }
                div { class: "set-card-heading",
                    h3 { class: "set-card-title", "{title}" }
                    if !desc.is_empty() {
                        p { class: "set-card-desc", "{desc}" }
                    }
                }
            }
            div { class: "set-card-body", {children} }
        }
    }
}

#[component]
pub fn IconBtn(
    icon: String,
    #[props(default)] title: String,
    #[props(default)] active: bool,
    #[props(default = 20)] size: u32,
    #[props(default)] class: String,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            class: "icon-btn {class}",
            class: if active { "active" },
            "data-el": "button",
            title: "{title}",
            onmousedown: move |e| e.stop_propagation(),
            // двойной клик по кнопке в заголовке не должен разворачивать окно
            ondoubleclick: move |e| e.stop_propagation(),
            onclick: move |e| {
                e.stop_propagation();
                onclick.call(e)
            },
            Icon { name: icon.clone(), size }
        }
    }
}

#[component]
pub fn Empty(icon: String, title: String, #[props(default)] text: String, children: Element) -> Element {
    rsx! {
        div { class: "empty",
            div { class: "empty-orb", Icon { name: icon.clone(), size: 34 } }
            h3 { "{title}" }
            if !text.is_empty() {
                p { "{text}" }
            }
            {children}
        }
    }
}

#[component]
pub fn Spinner() -> Element {
    rsx! {
        div { class: "spinner", span {} span {} span {} }
    }
}
