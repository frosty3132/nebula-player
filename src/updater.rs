//! Обновления из релизов GitHub: при запуске смотрим последний релиз,
//! и если он новее, предлагаем «Обновить». По кнопке скачиваем архив
//! релиза, достаём из него Nebula.exe и подменяем свой exe — Windows не даёт
//! перезаписать запущенный файл, но даёт переименовать его, поэтому старый
//! становится `*.old.exe` (удалим при следующем запуске), а новый встаёт на
//! его место. Потом приложение перезапускается само.

use std::path::{Path, PathBuf};
use std::time::Duration;

const REPO: &str = "frosty3132/nebula-player";
/// Флаг перезапуска после обновления: новая копия ждёт, пока старая выйдет.
pub const AFTER_UPDATE_ARG: &str = "--after-update";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    /// Версия без «v»: "0.4.1".
    pub version: String,
    /// Что нового (тело релиза, markdown как есть).
    pub notes: String,
    /// Страница релиза — если обновиться само не вышло.
    pub page: String,
    /// Архив с Nebula.exe или сам exe.
    pub asset_url: String,
    pub asset_size: u64,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("Nebula-Player/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}

/// "v0.4.1" / "0.4.1-beta" → [0, 4, 1].
fn parse_version(s: &str) -> Vec<u64> {
    s.trim()
        .trim_start_matches(['v', 'V'])
        .split(|c: char| c == '-' || c == '+')
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse::<u64>().unwrap_or(0))
        .collect()
}

/// `a` новее `b`?
pub fn is_newer(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (parse_version(a), parse_version(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x > y
}

pub fn current_version() -> &'static str {
    // только отладочная сборка: притвориться старой версией для проверки обновления
    #[cfg(debug_assertions)]
    {
        use std::sync::OnceLock;
        static FAKE: OnceLock<Option<String>> = OnceLock::new();
        if let Some(v) = FAKE.get_or_init(|| std::env::var("NEBULA_UPDATE_TEST_VERSION").ok()).as_deref() {
            return v;
        }
    }
    env!("CARGO_PKG_VERSION")
}

/// Последний релиз, если он новее запущенной версии.
pub async fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client()?
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Нет связи с GitHub: {e}"))?;
    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GitHub ответил {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if v["draft"].as_bool().unwrap_or(false) || v["prerelease"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    let tag = v["tag_name"].as_str().unwrap_or_default().to_string();
    if tag.is_empty() || !is_newer(&tag, current_version()) {
        return Ok(None);
    }
    // zip с Nebula.exe внутри втрое меньше самого exe — берём его, иначе exe
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let pick = |ext: &str| {
        assets.iter().find(|a| a["name"].as_str().is_some_and(|n| n.to_ascii_lowercase().ends_with(ext))).map(|a| {
            (a["browser_download_url"].as_str().unwrap_or_default().to_string(), a["size"].as_u64().unwrap_or(0))
        })
    };
    let Some((asset_url, asset_size)) = pick(".zip").or_else(|| pick(".exe")).filter(|(u, _)| !u.is_empty()) else {
        return Ok(None);
    };
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        notes: v["body"].as_str().unwrap_or_default().trim().to_string(),
        page: v["html_url"].as_str().unwrap_or_default().to_string(),
        asset_url,
        asset_size,
    }))
}

/// Скачать релиз и поставить новый exe на место текущего. `progress` — доля 0..1.
/// Возвращает путь к exe, который надо запустить.
pub async fn install(rel: &Release, progress: impl Fn(f32) + Send) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("Не найдена папка программы")?.to_path_buf();

    let mut resp = client()?
        .get(&rel.asset_url)
        .timeout(Duration::from_secs(600))
        .send()
        .await
        .map_err(|e| format!("Не удалось скачать: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Не удалось скачать: GitHub ответил {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(rel.asset_size).max(1);
    let mut data: Vec<u8> = Vec::with_capacity(total as usize);
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Загрузка прервалась: {e}"))? {
        data.extend_from_slice(&chunk);
        progress((data.len() as f64 / total as f64).min(1.0) as f32 * 0.95);
    }

    let is_zip = rel.asset_url.to_ascii_lowercase().ends_with(".zip");
    // архив ещё понадобится для библиотек, а сам exe можно забрать целиком
    let new_exe = if is_zip { exe_from_zip(&data)? } else { std::mem::take(&mut data) };
    // exe Windows начинается с «MZ» — не подменяем себя чем-то другим
    if new_exe.len() < 1024 * 1024 || !new_exe.starts_with(b"MZ") {
        return Err("В релизе не нашлось рабочего Nebula.exe".into());
    }
    progress(0.97);
    replace_exe(&exe, &dir, &new_exe)?;
    if is_zip {
        add_missing_dlls(&data, &dir);
    }
    progress(1.0);
    Ok(exe)
}

/// Положить рядом с exe библиотеки из архива, которых там нет (например,
/// WebView2Loader.dll). Уже лежащие не трогаем — запущенная программа их держит.
fn add_missing_dlls(data: &[u8], dir: &Path) {
    use std::io::Read;
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(data)) else { return };
    for i in 0..zip.len() {
        let Ok(mut f) = zip.by_index(i) else { continue };
        let name = f.name().replace('\\', "/");
        let Some(file) = name.rsplit('/').next().filter(|n| n.to_ascii_lowercase().ends_with(".dll")).map(str::to_string) else { continue };
        // только простое имя файла: никаких путей из архива
        if file.contains("..") || file.is_empty() {
            continue;
        }
        let dst = dir.join(&file);
        if dst.exists() {
            continue;
        }
        let mut buf = Vec::new();
        if f.read_to_end(&mut buf).is_ok() && std::fs::write(&dst, &buf).is_ok() {
            tracing::info!("обновление: добавлен {file}");
        }
    }
}

fn exe_from_zip(data: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(|e| format!("Архив релиза повреждён: {e}"))?;
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let name = names
        .iter()
        .find(|n| n.replace('\\', "/").rsplit('/').next().is_some_and(|f| f.eq_ignore_ascii_case("Nebula.exe")))
        .or_else(|| names.iter().find(|n| n.to_ascii_lowercase().ends_with(".exe")))
        .ok_or("В архиве релиза нет Nebula.exe")?
        .clone();
    let mut f = zip.by_name(&name).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

fn old_path(exe: &Path) -> PathBuf {
    exe.with_extension("old.exe")
}

fn replace_exe(exe: &Path, dir: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = dir.join("Nebula.update.exe");
    std::fs::write(&tmp, bytes).map_err(|e| {
        format!("Нет доступа к папке программы ({}): {e}. Скачайте обновление со страницы релиза", dir.display())
    })?;
    let old = old_path(exe);
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Не удалось заменить программу: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, exe) {
        // вернуть всё как было, чтобы не остаться без exe
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Не удалось заменить программу: {e}"));
    }
    Ok(())
}

/// Запустить обновлённую программу (она подождёт, пока эта копия выйдет).
pub fn relaunch(exe: &Path) -> Result<(), String> {
    std::process::Command::new(exe).arg(AFTER_UPDATE_ARG).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Удалить exe, оставшийся от прошлого обновления. Старая копия могла ещё не
/// до конца выйти — пробуем несколько раз в фоне.
pub fn cleanup_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = old_path(&exe);
    if !old.exists() {
        return;
    }
    let _ = std::thread::Builder::new().name("nebula-update-cleanup".into()).spawn(move || {
        for _ in 0..30 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                tracing::info!("обновление: удалён старый exe");
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("v0.4.1", "0.4.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0", "0.99.99"));
        assert!(!is_newer("v0.4.0", "0.4.0"));
        assert!(!is_newer("0.4", "0.4.0"));
        assert!(!is_newer("v0.3.9", "0.4.0"));
        assert!(!is_newer("0.4.0-beta", "0.4.0"));
    }

    #[test]
    fn exe_found_in_zip() {
        use std::io::Write;
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("Nebula/readme.txt", o).unwrap();
            z.write_all(b"hi").unwrap();
            z.start_file("Nebula/Nebula.exe", o).unwrap();
            z.write_all(b"MZ-test").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(exe_from_zip(buf.get_ref()).unwrap(), b"MZ-test");
    }
}
