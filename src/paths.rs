//! Каталоги приложения и атомарная запись JSON.
//!
//! По умолчанию: настройки в `%APPDATA%\NebulaPlayer`, кэш в `%LOCALAPPDATA%\NebulaPlayer`.
//! Портативный режим: положите рядом с exe пустой файл `portable.txt` —
//! всё будет храниться в папке `data` рядом с программой.

use serde::{de::DeserializeOwned, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub struct Paths {
    pub config: PathBuf,
    pub themes: PathBuf,
    pub cache: PathBuf,
    pub images: PathBuf,
    pub covers: PathBuf,
    pub lyrics: PathBuf,
    pub webview: PathBuf,
    pub portable: bool,
}

static PATHS: OnceLock<Paths> = OnceLock::new();

pub fn init() -> &'static Paths {
    PATHS.get_or_init(|| {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let portable = exe_dir.as_ref().is_some_and(|d| d.join("portable.txt").exists());
        let (config, cache) = match (&exe_dir, portable) {
            (Some(dir), true) => (dir.join("data").join("config"), dir.join("data").join("cache")),
            _ => (
                dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("NebulaPlayer"),
                dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".")).join("NebulaPlayer"),
            ),
        };
        let p = Paths {
            themes: config.join("themes"),
            images: cache.join("images"),
            covers: cache.join("covers"),
            lyrics: cache.join("lyrics"),
            webview: cache.join("webview"),
            config,
            cache,
            portable,
        };
        for d in [&p.config, &p.themes, &p.cache, &p.images, &p.covers, &p.lyrics, &p.webview] {
            let _ = std::fs::create_dir_all(d);
        }
        p
    })
}

pub fn paths() -> &'static Paths {
    PATHS.get().unwrap_or_else(|| init())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    match serde_json::from_slice(&data) {
        Ok(v) => Some(v),
        Err(e) => {
            tracing::warn!("не удалось разобрать {}: {e}", path.display());
            None
        }
    }
}

/// Пишем во временный файл и переименовываем — файл не побьётся при сбое.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn hash_key(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let d = Sha256::digest(s.as_bytes());
    d.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
