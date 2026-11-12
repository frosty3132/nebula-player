//! Системные возможности для островка: буфер обмена (чтение, запись,
//! история), снимок экрана, пипетка, питание компьютера, тёмная тема Windows,
//! «кто-то на весь экран» и погода. Всё на Win32 без лишних зависимостей;
//! на других системах функции ничего не делают.

#![allow(dead_code)]

/// Что лежит в буфере обмена.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipData {
    Text(String),
    /// Изображение: размер и сами данные (CF_DIB), чтобы вернуть его в буфер.
    Image { w: u32, h: u32, dib: std::sync::Arc<Vec<u8>> },
}

#[cfg(windows)]
mod imp {
    use super::ClipData;
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::{HWND, POINT};
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetDC, GetPixel, ReleaseDC, SelectObject, BITMAPINFOHEADER, CAPTUREBLT, SRCCOPY,
    };
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::System::Ole::{CF_BITMAP, CF_DIB, CF_UNICODETEXT};
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    pub fn clipboard_seq() -> u32 {
        // SAFETY: без аргументов, только читает счётчик
        unsafe { GetClipboardSequenceNumber() }
    }

    /// Буфер может быть занят другим приложением — пробуем несколько раз.
    fn open() -> bool {
        for _ in 0..8 {
            // SAFETY: открываем буфер без окна-владельца
            if unsafe { OpenClipboard(std::ptr::null_mut::<core::ffi::c_void>() as HWND) } != 0 {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        false
    }

    pub fn read() -> Option<ClipData> {
        if !open() {
            return None;
        }
        // SAFETY: буфер открыт; память из GetClipboardData читаем под GlobalLock
        let out = unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) != 0 {
                let h = GetClipboardData(CF_UNICODETEXT as u32);
                let p = GlobalLock(h) as *const u16;
                let r = if p.is_null() {
                    None
                } else {
                    let max = GlobalSize(h) / 2;
                    let mut n = 0;
                    while n < max && *p.add(n) != 0 {
                        n += 1;
                    }
                    let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
                    GlobalUnlock(h);
                    Some(ClipData::Text(s))
                };
                r
            } else if IsClipboardFormatAvailable(CF_DIB as u32) != 0 {
                let h = GetClipboardData(CF_DIB as u32);
                let p = GlobalLock(h) as *const u8;
                if p.is_null() {
                    None
                } else {
                    let size = GlobalSize(h);
                    let bytes = std::slice::from_raw_parts(p, size).to_vec();
                    GlobalUnlock(h);
                    if size >= std::mem::size_of::<BITMAPINFOHEADER>() {
                        let hdr = &*(bytes.as_ptr() as *const BITMAPINFOHEADER);
                        Some(ClipData::Image { w: hdr.biWidth.unsigned_abs(), h: hdr.biHeight.unsigned_abs(), dib: Arc::new(bytes) })
                    } else {
                        None
                    }
                }
            } else {
                None
            }
        };
        // SAFETY: закрываем открытый выше буфер
        unsafe { CloseClipboard() };
        out
    }

    fn put(format: u32, bytes: &[u8]) -> bool {
        if !open() {
            return false;
        }
        // SAFETY: буфер открыт; память выделяем и передаём системе (после
        // SetClipboardData она принадлежит буферу)
        let ok = unsafe {
            EmptyClipboard();
            let h = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
            if h.is_null() {
                false
            } else {
                let p = GlobalLock(h) as *mut u8;
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                GlobalUnlock(h);
                !SetClipboardData(format, h).is_null()
            }
        };
        // SAFETY: закрываем открытый выше буфер
        unsafe { CloseClipboard() };
        ok
    }

    pub fn write_text(s: &str) -> bool {
        let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 2) };
        put(CF_UNICODETEXT as u32, bytes)
    }

    pub fn write_dib(dib: &[u8]) -> bool {
        put(CF_DIB as u32, dib)
    }

    /// Снимок прямоугольника экрана в буфер обмена; возвращает размер.
    pub fn screenshot(x: i32, y: i32, w: i32, h: i32) -> Option<(u32, u32)> {
        // SAFETY: обычная последовательность GDI: DC экрана → совместимый
        // битмап → BitBlt; битмап отдаём буферу, DC освобождаем
        unsafe {
            let screen = GetDC(std::ptr::null_mut());
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp as _);
            BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT);
            SelectObject(mem, old);
            DeleteDC(mem);
            ReleaseDC(std::ptr::null_mut(), screen);
            if !open() {
                return None;
            }
            EmptyClipboard();
            let ok = !SetClipboardData(CF_BITMAP as u32, bmp as _).is_null();
            CloseClipboard();
            ok.then_some((w as u32, h as u32))
        }
    }

    /// Цвет пикселя под курсором (#RRGGBB).
    pub fn pixel_at_cursor() -> Option<String> {
        // SAFETY: читаем позицию курсора и один пиксель экрана
        unsafe {
            let mut p = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut p) == 0 {
                return None;
            }
            let dc = GetDC(std::ptr::null_mut());
            let c = GetPixel(dc, p.x, p.y);
            ReleaseDC(std::ptr::null_mut(), dc);
            if c == u32::MAX {
                return None;
            }
            Some(format!("#{:02X}{:02X}{:02X}", c & 0xFF, (c >> 8) & 0xFF, (c >> 16) & 0xFF))
        }
    }

    pub fn lock() {
        // SAFETY: без аргументов
        unsafe { windows_sys::Win32::System::Shutdown::LockWorkStation() };
    }

    pub fn sleep() {
        // SAFETY: обычный сон (не гибернация)
        unsafe { windows_sys::Win32::System::Power::SetSuspendState(false, false, false) };
    }

    fn run(cmd: &str, args: &[&str]) {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new(cmd).args(args).creation_flags(0x0800_0000).spawn();
    }

    pub fn restart() {
        run("shutdown", &["/r", "/t", "0"]);
    }

    pub fn shutdown() {
        run("shutdown", &["/s", "/t", "0"]);
    }

    const THEME_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Тёмная тема Windows включена?
    pub fn dark_theme() -> bool {
        use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
        let mut v: u32 = 1;
        let mut size = 4u32;
        // SAFETY: читаем один DWORD из реестра пользователя
        let r = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                wide(THEME_KEY).as_ptr(),
                wide("AppsUseLightTheme").as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut v as *mut u32).cast(),
                &mut size,
            )
        };
        r == 0 && v == 0
    }

    /// Переключить светлую/тёмную тему Windows (приложения и система).
    pub fn set_dark_theme(dark: bool) {
        use windows_sys::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
        let v: u32 = if dark { 0 } else { 1 };
        for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
            // SAFETY: пишем один DWORD в реестр пользователя
            unsafe {
                RegSetKeyValueW(HKEY_CURRENT_USER, wide(THEME_KEY).as_ptr(), wide(name).as_ptr(), REG_DWORD, (&v as *const u32).cast(), 4);
            }
        }
        // сообщаем окнам, что тема поменялась
        let what = wide("ImmersiveColorSet");
        // SAFETY: широковещательное уведомление с таймаутом
        unsafe {
            SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, what.as_ptr() as isize, SMTO_ABORTIFHUNG, 300, std::ptr::null_mut());
        }
    }

    /// Сейчас кто-то на весь экран (кино, игра, презентация)?
    pub fn fullscreen_busy() -> bool {
        use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
        let mut st = 0;
        // SAFETY: функция заполняет одно число
        let hr = unsafe { SHQueryUserNotificationState(&mut st) };
        hr == 0 && matches!(st, 2..=4)
    }

    pub fn beep() {
        use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
        use windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONEXCLAMATION;
        // SAFETY: системный звук
        unsafe { MessageBeep(MB_ICONEXCLAMATION) };
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ClipData;
    pub fn clipboard_seq() -> u32 {
        0
    }
    pub fn read() -> Option<ClipData> {
        None
    }
    pub fn write_text(_: &str) -> bool {
        false
    }
    pub fn write_dib(_: &[u8]) -> bool {
        false
    }
    pub fn screenshot(_: i32, _: i32, _: i32, _: i32) -> Option<(u32, u32)> {
        None
    }
    pub fn pixel_at_cursor() -> Option<String> {
        None
    }
    pub fn lock() {}
    pub fn sleep() {}
    pub fn restart() {}
    pub fn shutdown() {}
    pub fn dark_theme() -> bool {
        true
    }
    pub fn set_dark_theme(_: bool) {}
    pub fn fullscreen_busy() -> bool {
        false
    }
    pub fn beep() {}
}

pub use imp::*;

/// «Чистый текст»: убрать невидимые символы, лишние пробелы и пустые строки.
pub fn clean_text(s: &str) -> String {
    let no_zw: String = s.chars().filter(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}')).map(|c| if c == '\u{00A0}' { ' ' } else { c }).collect();
    let mut out: Vec<String> = Vec::new();
    for line in no_zw.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() && out.last().is_some_and(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
    }
    out.join("\n").trim().to_string()
}

/// Погода: город → (иконка, температура). Open-Meteo, без ключей.
pub async fn weather(city: &str) -> Option<(&'static str, i32)> {
    let c = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().ok()?;
    let geo: serde_json::Value = c
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query(&[("name", city), ("count", "1"), ("language", "ru")])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let r = geo.get("results")?.get(0)?;
    let (lat, lon) = (r.get("latitude")?.as_f64()?, r.get("longitude")?.as_f64()?);
    let w: serde_json::Value = c
        .get("https://api.open-meteo.com/v1/forecast")
        .query(&[("latitude", lat.to_string()), ("longitude", lon.to_string()), ("current", "temperature_2m,weather_code,is_day".to_string())])
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let cur = w.get("current")?;
    let t = cur.get("temperature_2m")?.as_f64()?.round() as i32;
    let code = cur.get("weather_code")?.as_i64()?;
    let day = cur.get("is_day").and_then(|d| d.as_i64()).unwrap_or(1) == 1;
    let icon = match code {
        0 => if day { "☀️" } else { "🌙" },
        1 | 2 => if day { "🌤️" } else { "☁️" },
        3 => "☁️",
        45 | 48 => "🌫️",
        51..=67 | 80..=82 => "🌧️",
        71..=77 | 85 | 86 => "❄️",
        95..=99 => "⛈️",
        _ => "🌡️",
    };
    Some((icon, t))
}

#[cfg(test)]
mod tests {
    use super::clean_text;

    #[test]
    fn clean_text_collapses_spaces_and_invisible_chars() {
        assert_eq!(clean_text("  Привет,\u{200B}   мир!  \n\n\n  второй\u{00A0}абзац "), "Привет, мир!\n\nвторой абзац");
    }
}
