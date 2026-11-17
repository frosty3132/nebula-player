//! Только одна копия Nebula. Повторный запуск (ярлык, автозагрузка, двойной
//! щелчок по exe) не открывает второе приложение со своим значком в трее, а
//! будит уже запущенное: оно показывает окно и завершается само.
//!
//! Как устроено: первая копия держит именованный мьютекс и ждёт именованное
//! событие в отдельном потоке. Следующая копия видит, что мьютекс уже есть,
//! взводит событие и выходит. Отладочная сборка пользуется своими именами,
//! чтобы не мешать установленной Nebula.

use std::sync::atomic::{AtomicBool, Ordering};

/// Другая копия попросила показать окно — интерфейс забирает просьбу по таймеру.
static SHOW_REQUEST: AtomicBool = AtomicBool::new(false);

/// Забрать просьбу «покажи окно» (true — была).
pub fn take_show_request() -> bool {
    SHOW_REQUEST.swap(false, Ordering::Relaxed)
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn names() -> (Vec<u16>, Vec<u16>) {
    let suffix = if cfg!(debug_assertions) { ".Debug" } else { "" };
    (wide(&format!("Local\\Nebula.Player.Instance{suffix}")), wide(&format!("Local\\Nebula.Player.Show{suffix}")))
}

/// Стать единственной копией. `false` — Nebula уже запущена: её попросили
/// показать окно, а этому процессу надо просто выйти.
#[cfg(windows)]
pub fn acquire() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    let (mutex_name, event_name) = names();
    // после обновления старая копия ещё выходит — ждём её, а не будим
    let after_update = std::env::args().any(|a| a == crate::updater::AFTER_UPDATE_ARG);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let (mutex, exists) = loop {
        // SAFETY: имя — нуль-терминированная UTF-16 строка; дескриптор держим до
        // конца процесса (его закроет система), поэтому не закрываем
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        // SAFETY: без аргументов, читает код ошибки последнего вызова в этом потоке
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if exists && after_update && !mutex.is_null() && std::time::Instant::now() < deadline {
            // SAFETY: свой дескриптор, больше не нужен
            unsafe { CloseHandle(mutex) };
            std::thread::sleep(std::time::Duration::from_millis(200));
            continue;
        }
        break (mutex, exists);
    };
    if mutex.is_null() {
        // мьютекс не создать (странные права) — лучше запуститься, чем молча не открыться
        tracing::warn!("одна копия: мьютекс не создан, запускаемся без проверки");
        return true;
    }
    if exists {
        // SAFETY: имя события — нуль-терминированная строка; дескриптор закрывает система при выходе
        unsafe {
            let ev = OpenEventW(EVENT_MODIFY_STATE, 0, event_name.as_ptr());
            if !ev.is_null() {
                // окно первой копии сможет выйти на передний план
                AllowSetForegroundWindow(ASFW_ANY);
                SetEvent(ev);
            }
        }
        tracing::info!("Nebula уже запущена — показываем её окно и выходим");
        return false;
    }
    // SAFETY: автосбрасываемое событие с именем; дескриптор живёт до конца процесса
    let ev = unsafe { CreateEventW(std::ptr::null(), 0, 0, event_name.as_ptr()) };
    if !ev.is_null() {
        let ev = ev as usize;
        let _ = std::thread::Builder::new().name("nebula-instance".into()).spawn(move || loop {
            // SAFETY: дескриптор события действителен, пока жив процесс
            let r = unsafe { WaitForSingleObject(ev as _, INFINITE) };
            if r != WAIT_OBJECT_0 {
                break;
            }
            SHOW_REQUEST.store(true, Ordering::Relaxed);
        });
    }
    true
}

#[cfg(not(windows))]
pub fn acquire() -> bool {
    true
}
