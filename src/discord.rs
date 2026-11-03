//! Optional Discord Rich Presence integration over the local Discord IPC.

use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::{mpsc, OnceLock};

enum Command {
    Set { title: String, artist: String, paused: bool },
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

pub fn set_activity(title: &str, artist: &str, paused: bool) {
    if let Some(tx) = RPC_TX.get() {
        let _ = tx.send(Command::Set { title: title.into(), artist: artist.into(), paused });
    }
}

pub fn clear() {
    if let Some(tx) = RPC_TX.get() {
        let _ = tx.send(Command::Clear);
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
            Command::Set { title, artist, paused } => {
                if client.is_none() {
                    let mut c = DiscordIpcClient::new(&client_id);
                    if let Err(e) = c.connect() {
                        tracing::debug!("Discord RPC недоступен: {e}");
                        continue;
                    }
                    client = Some(c);
                }
                let activity = activity::Activity::new()
                    .name("Nebula")
                    .details(title)
                    .state(if paused { format!("Пауза · {artist}") } else { artist });
                if client.as_mut().is_some_and(|c| c.set_activity(activity).is_err()) {
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
