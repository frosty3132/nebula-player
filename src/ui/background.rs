//! Фон окна: звёздное небо с туманностями, градиент, картинка/GIF, видео
//! или размытая обложка. Звёзды — это `box-shadow` на трёх слоях, которые
//! двигаются через `transform`: анимация целиком на GPU-композиторе, CPU не нагружается.

use super::state::app_state;
use crate::model::{file_url, image_url, ArtSize};
use crate::theme::{BgFit, BgKind};
use dioxus::prelude::*;

fn lcg(seed: &mut u64) -> f32 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 33) as f32) / (u32::MAX >> 1) as f32
}

/// Слой звёзд: список теней «x y 0 0 rgba(...)» в квадрате 2000×2000.
pub fn star_layer(count: usize, seed: u64, max_alpha: f32) -> String {
    let mut s = seed;
    let mut out = String::with_capacity(count * 28);
    for i in 0..count {
        let x = (lcg(&mut s) * 2000.0) as u32;
        let y = (lcg(&mut s) * 2000.0) as u32;
        let a = 0.25 + lcg(&mut s) * (max_alpha - 0.25);
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{x}px {y}px 0 0 rgba(var(--star-rgb),{a:.2})"));
    }
    out
}

fn source_url(src: &str) -> String {
    let s = src.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        image_url(s)
    } else {
        file_url(s)
    }
}

#[component]
pub fn Background() -> Element {
    let st = app_state();
    let theme = st.theme.read();
    let bg = theme.background.clone();
    let stars = theme.stars.clone();
    drop(theme);

    let low = st.settings.read().low_power;
    let density = if low { stars.density * 0.5 } else { stars.density };
    // тени пересчитываются только при смене плотности звёзд
    let layers = use_memo(move || {
        let raw = st.theme.read().stars.density;
        let d = if st.settings.read().low_power { raw * 0.5 } else { raw }.clamp(0.0, 3.0);
        [
            star_layer((420.0 * d) as usize, 7, 0.75),
            star_layer((160.0 * d) as usize, 13, 0.9),
            star_layer((60.0 * d) as usize, 29, 1.0),
        ]
    });
    let layers = layers.read();

    let fit = match bg.fit {
        BgFit::Cover => "cover",
        BgFit::Contain => "contain",
        BgFit::Tile => "auto",
        BgFit::Stretch => "100% 100%",
    };
    let repeat = if bg.fit == BgFit::Tile { "repeat" } else { "no-repeat" };
    let show_space = matches!(bg.kind, BgKind::Space) || bg.stars_over_media;
    let show_nebula = bg.nebula && matches!(bg.kind, BgKind::Space | BgKind::Gradient);

    let cover = if bg.kind == BgKind::Cover {
        st.now.read().as_ref().and_then(|t| t.art(ArtSize::Large))
    } else {
        None
    };

    rsx! {
        div { class: "bg", "data-el": "background",
            div { class: "bg-base bg-{bg.kind:?}" }
            if show_nebula {
                div { class: "nebula",
                    span { class: "neb n1" }
                    span { class: "neb n2" }
                    span { class: "neb n3" }
                }
            }
            match bg.kind {
                BgKind::Image if !bg.source.trim().is_empty() => rsx! {
                    div {
                        class: "bg-media",
                        style: "background-image: url('{source_url(&bg.source)}'); background-size: {fit}; background-repeat: {repeat};",
                    }
                },
                BgKind::Video if !bg.source.trim().is_empty() => rsx! {
                    video {
                        class: "bg-media bg-video",
                        src: "{source_url(&bg.source)}",
                        autoplay: true,
                        muted: true,
                        "loop": true,
                        "playsinline": true,
                        style: "object-fit: {fit};",
                    }
                },
                BgKind::Cover => rsx! {
                    if let Some(c) = cover {
                        div { class: "bg-media bg-cover", style: "background-image: url('{c}');" }
                    }
                },
                _ => rsx! {},
            }
            if show_space && density > 0.0 {
                div { class: if stars.twinkle { "stars twinkle" } else { "stars" },
                    div { class: "star-layer l1", style: "box-shadow: {layers[0]};" }
                    div { class: "star-layer l2", style: "box-shadow: {layers[1]};" }
                    div { class: "star-layer l3", style: "box-shadow: {layers[2]};" }
                }
                if stars.shooting && !low {
                    div { class: "shooting",
                        span { class: "shoot s1" }
                        span { class: "shoot s2" }
                        span { class: "shoot s3" }
                    }
                }
            }
            div { class: "bg-dim" }
        }
    }
}
