//! Системное для островка: «кто-то сейчас на весь экран» (кино, игра,
//! презентация) — тогда островок прячется за край экрана.

/// Сейчас кто-то на весь экран?
#[cfg(windows)]
pub fn fullscreen_busy() -> bool {
    use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
    let mut st = 0;
    // SAFETY: функция заполняет одно число
    let hr = unsafe { SHQueryUserNotificationState(&mut st) };
    hr == 0 && matches!(st, 2..=4)
}

#[cfg(not(windows))]
pub fn fullscreen_busy() -> bool {
    false
}
