//! На тулчейне `*-windows-gnu` WebView2 линкуется динамически с `WebView2Loader.dll`,
//! поэтому DLL должна лежать рядом с exe. Копируем её в каталог профиля сборки
//! (`target/debug`, `target/release`), чтобы `cargo run` работал без ручных действий.
//! На MSVC-тулчейне загрузчик линкуется статически и копирование не нужно.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor");

    let target = std::env::var("TARGET").unwrap_or_default();
    if !(target.contains("windows") && target.contains("gnu")) {
        return;
    }

    let arch = if target.starts_with("x86_64") {
        "x64"
    } else if target.starts_with("aarch64") {
        "arm64"
    } else {
        "x86"
    };

    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let src = manifest.join("vendor").join(arch).join("WebView2Loader.dll");
    if !src.exists() {
        println!("cargo:warning=vendor/{arch}/WebView2Loader.dll не найден — exe не запустится без него");
        return;
    }

    // OUT_DIR = target/<profile>/build/<pkg>-<hash>/out → поднимаемся до target/<profile>
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    if let Some(profile_dir) = out.ancestors().nth(3) {
        for dir in [profile_dir.to_path_buf(), profile_dir.join("deps"), profile_dir.join("examples")] {
            if dir.is_dir() {
                let _ = std::fs::copy(&src, dir.join("WebView2Loader.dll"));
            }
        }
    }
}
