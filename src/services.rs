//! Глобальные сервисы: tokio-рантайм, HTTP-клиент, SoundCloud, YouTube, тексты, медиа и плеер.
//! Создаются один раз в `main` до запуска окна.

use crate::audio::Player;
use crate::lyrics::LyricsClient;
use crate::media::Media;
use crate::settings::Settings;
use crate::soundcloud::SoundCloud;
use crate::spotify::Spotify;
use crate::yandex::YandexMusic;
use crate::youtube::YouTube;
use std::sync::OnceLock;
use std::time::Duration;

pub struct Services {
    pub rt: tokio::runtime::Handle,
    pub sc: SoundCloud,
    pub yandex: YandexMusic,
    pub yt: YouTube,
    pub spotify: Spotify,
    pub lyrics: LyricsClient,
    pub media: Media,
    pub player: Player,
}

static SERVICES: OnceLock<Services> = OnceLock::new();

pub fn init(rt: tokio::runtime::Handle, settings: &Settings) -> &'static Services {
    SERVICES.get_or_init(|| {
        let http = reqwest::Client::builder()
            .user_agent(concat!("NebulaPlayer/", env!("CARGO_PKG_VERSION"), " (Windows; desktop music player)"))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("HTTP-клиент");
        let player = Player::new(http.clone(), rt.clone(), settings.dsp(), settings.output_device.clone());
        player.set_read_ahead(settings.read_ahead_s as u64 * 1000);
        let media = Media::new(http.clone());
        media.set_limit_mb(settings.image_cache_mb);
        Services {
            sc: SoundCloud::new(http.clone()),
            yandex: YandexMusic::new(http.clone()),
            yt: YouTube::new(http.clone()),
            spotify: Spotify::new(http.clone()),
            lyrics: LyricsClient::new(http.clone()),
            media,
            player,
            rt,
        }
    })
}

pub fn svc() -> &'static Services {
    SERVICES.get().expect("services::init не вызван")
}
