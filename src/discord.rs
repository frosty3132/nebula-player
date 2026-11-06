//! Discord Rich Presence через локальный IPC Discord.
//!
//! Статус «Слушает»: название, исполнитель, обложка трека и полоса времени
//! (Discord сам двигает секунды по отметкам начала и конца), а на паузе —
//! текущая позиция текстом.

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::{mpsc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Встроенное приложение Discord для статуса Nebula.
pub const DEFAULT_CLIENT_ID: &str = "1568072624427241555";

/// Что показать в статусе.
#[derive(Clone, Debug, PartialEq)]
pub struct Presence {
    pub title: String,
    pub artist: String,
    pub paused: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
    /// Обложка по https (Discord сам подгружает внешние картинки).
    pub cover: Option<String>,
    /// Ссылка на трек — кнопка «Открыть в SoundCloud».
    pub url: Option<String>,
}

enum Command {
    Set(Presence),
    Clear,
}

static RPC_TX: OnceLock<mpsc::Sender<Command>> = OnceLock::new();

pub fn init(client_id: &str) {
    let client_id = client_id.trim().to_string();
    if client_id.is_empty() || RPC_TX.get().is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel::<Command>();
    let _ = RPC_TX.set(tx);
    std::thread::Builder::new()
        .name("nebula-discord-rpc".into())
        .spawn(move || worker(client_id, rx))
        .ok();
}

pub fn set_activity(p: Presence) {
    if let Some(tx) = RPC_TX.get() {
        let _ = tx.send(Command::Set(p));
    }
}

pub fn clear() {
    if let Some(tx) = RPC_TX.get() {
        let _ = tx.send(Command::Clear);
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn mmss(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Отметки начала и конца (мс Unix) для полосы времени в Discord.
pub fn timestamps(position_ms: u64, duration_ms: u64, now: i64) -> (i64, Option<i64>) {
    let start = now - position_ms as i64;
    (start, (duration_ms > 0).then(|| start + duration_ms as i64))
}

/// Discord требует от двух символов в текстовых полях.
fn field(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() < 2 {
        format!("{s}  ")
    } else {
        s.chars().take(128).collect()
    }
}

fn worker(client_id: String, rx: mpsc::Receiver<Command>) {
    let mut client: Option<DiscordIpcClient> = None;
    while let Ok(command) = rx.recv() {
        match command {
            Command::Clear => {
                if let Some(c) = client.as_mut() {
                    let _ = c.clear_activity();
                }
            }
            Command::Set(p) => {
                if client.is_none() {
                    let mut c = DiscordIpcClient::new(&client_id);
                    if let Err(e) = c.connect() {
                        tracing::debug!("Discord RPC недоступен: {e}");
                        continue;
                    }
                    client = Some(c);
                }
                let title = field(&p.title);
                let state = if p.paused {
                    let pos = if p.duration_ms > 0 { format!("{} / {}", mmss(p.position_ms), mmss(p.duration_ms)) } else { mmss(p.position_ms) };
                    field(&format!("{} · на паузе {pos}", p.artist))
                } else {
                    field(&p.artist)
                };
                let mut act = activity::Activity::new()
                    .name("Nebula")
                    .activity_type(activity::ActivityType::Listening)
                    .status_display_type(activity::StatusDisplayType::Details)
                    .details(title)
                    .state(state);
                // только картинка, без подписи: подпись Discord показывает третьей
                // строкой, а без обложки блок картинки вообще не добавляем
                // (иначе вместо неё рисуется знак вопроса)
                if let Some(c) = &p.cover {
                    act = act.assets(activity::Assets::new().large_image(c.clone()));
                }
                if !p.paused {
                    let (start, end) = timestamps(p.position_ms, p.duration_ms, now_ms());
                    let mut ts = activity::Timestamps::new().start(start);
                    if let Some(e) = end {
                        ts = ts.end(e);
                    }
                    act = act.timestamps(ts);
                }
                if let Some(u) = &p.url {
                    act = act.buttons(vec![activity::Button::new("Открыть в SoundCloud", u.clone())]);
                }
                if client.as_mut().is_some_and(|c| c.set_activity(act).is_err()) {
                    client = None;
                }
            }
        }
    }
    if let Some(mut c) = client {
        let _ = c.clear_activity();
        let _ = c.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_timestamps_follow_position() {
        let (start, end) = timestamps(30_000, 180_000, 1_000_000);
        assert_eq!(start, 970_000);
        assert_eq!(end, Some(1_150_000));
        assert_eq!(timestamps(5_000, 0, 10_000).1, None);
        assert_eq!(mmss(83_000), "1:23");
    }
}
