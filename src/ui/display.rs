//! Частота обновления активного монитора (Гц) для плавной синхронизации
//! визуализаторов, островка и JS-анимаций без лишней нагрузки на VDOM.

use dioxus::desktop::tao::window::Window;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

static APP_HZ: AtomicU32 = AtomicU32::new(60);
static ISLAND_HZ: AtomicU32 = AtomicU32::new(60);
static LOW_POWER: AtomicBool = AtomicBool::new(false);

pub fn set_low_power(v: bool) {
    LOW_POWER.store(v, Ordering::Relaxed);
}

pub fn low_power() -> bool {
    LOW_POWER.load(Ordering::Relaxed)
}

/// Переводит миллигерцы частоты монитора в целые Гц.
#[allow(dead_code)]
pub fn hz_from_millihertz(mh: u32) -> u32 {
    if mh == 0 {
        return 60;
    }
    mh.saturating_add(500).saturating_div(1000).clamp(30, 360)
}

/// Интервал одного кадра для заданной герцовки (в режиме экономии частоты — вдвое реже).
pub fn frame_interval(hz: u32, low_power: bool) -> Duration {
    let eff = if low_power { (hz / 2).clamp(24, 60) } else { hz.clamp(30, 360) };
    let ns = 1_000_000_000u64 / eff as u64;
    Duration::from_nanos(ns.max(2_500_000))
}

/// Читает реальную частоту обновления монитора, на котором сейчас находится HWND (Windows).
#[cfg(windows)]
pub fn refresh_hz_for_hwnd(hwnd: isize) -> Option<u32> {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS, MONITORINFOEXW,
        MONITOR_DEFAULTTONEAREST,
    };
    if hwnd == 0 {
        return None;
    }
    unsafe {
        let hmon = MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST);
        if hmon.is_null() {
            return None;
        }
        let mut mi: MONITORINFOEXW = std::mem::zeroed();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, (&raw mut mi).cast()) == 0 {
            return None;
        }
        let mut dm: DEVMODEW = std::mem::zeroed();
        dm.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
        if EnumDisplaySettingsW(mi.szDevice.as_ptr(), ENUM_CURRENT_SETTINGS, &mut dm) != 0
            && dm.dmDisplayFrequency > 1
        {
            return Some(dm.dmDisplayFrequency.clamp(30, 360));
        }
    }
    None
}

#[cfg(not(windows))]
pub fn refresh_hz_for_hwnd(_hwnd: isize) -> Option<u32> {
    None
}

/// Частота обновления монитора для окна (`EnumDisplaySettingsW`, иначе 60 Гц).
pub fn window_refresh_hz(win: &Window) -> u32 {
    #[cfg(windows)]
    {
        use dioxus::desktop::tao::platform::windows::WindowExtWindows;
        if let Some(hz) = refresh_hz_for_hwnd(win.hwnd()) {
            return hz;
        }
    }
    #[cfg(not(windows))]
    let _ = win;
    60
}

pub fn sync_app_hz(win: &Window) -> u32 {
    let hz = window_refresh_hz(win);
    APP_HZ.store(hz, Ordering::Relaxed);
    hz
}

pub fn app_hz() -> u32 {
    APP_HZ.load(Ordering::Relaxed).clamp(30, 360)
}

pub fn sync_island_hz(win: &Window) -> u32 {
    let hz = window_refresh_hz(win);
    ISLAND_HZ.store(hz, Ordering::Relaxed);
    hz
}

pub fn island_hz() -> u32 {
    ISLAND_HZ.load(Ordering::Relaxed).clamp(30, 360)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn millihertz_rounding_and_clamping() {
        assert_eq!(hz_from_millihertz(0), 60);
        assert_eq!(hz_from_millihertz(59_940), 60);
        assert_eq!(hz_from_millihertz(143_856), 144);
        assert_eq!(hz_from_millihertz(164_900), 165);
        assert_eq!(hz_from_millihertz(240_000), 240);
        assert_eq!(hz_from_millihertz(1_000_000), 360);
    }

    #[test]
    fn frame_interval_scales_with_hz() {
        let d60 = frame_interval(60, false);
        let d144 = frame_interval(144, false);
        let d240 = frame_interval(240, false);
        assert!((d60.as_secs_f64() - 1.0 / 60.0).abs() < 0.0005);
        assert!((d144.as_secs_f64() - 1.0 / 144.0).abs() < 0.0005);
        assert!((d240.as_secs_f64() - 1.0 / 240.0).abs() < 0.0005);
        assert!(frame_interval(144, true) > d144);
    }
}
