use std::{collections::VecDeque, sync::Arc, time::Duration};

use arc_swap::ArcSwapOption;
use atomic_float::AtomicF32;
use downloader::Downloader;
use reqwest::Client;
use rodio::{OutputStream, OutputStreamBuilder, Sink};
use tokio::{
    select,
    sync::{
        mpsc::{Receiver, Sender},
        RwLock,
    },
    task,
};

#[cfg(feature = "mpris")]
use mpris_server::{PlaybackStatus, PlayerInterface, Property};

use bytes::Bytes;
use crate::debug_log;

use crate::{
    messages::Message,
    player::{self, bookmark::Bookmarks, persistent_volume::PersistentVolume},
    tracks::{self, list::List},
    Args,
};

pub mod audio;
pub mod bookmark;
pub mod downloader;
pub mod error;
pub mod persistent_volume;
pub mod queue;
pub mod ui;

pub use error::Error;

#[cfg(feature = "mpris")]
pub mod mpris;

pub struct Player {

    pub sink: Arc<Sink>,

    pub buffer_size: usize,

    pub current: ArcSwapOption<tracks::Info>,

    pub progress: AtomicF32,

    pub tracks: RwLock<VecDeque<tracks::QueuedTrack>>,

    pub bookmarks: Bookmarks,

    timeout: Duration,

    pub list: List,

    volume: PersistentVolume,

    pub client: Client,

    #[cfg(feature = "color")]
    pub art_cache: Arc<ui::cover::ArtCache>,
    pub skip_art: bool,

    #[cfg(feature = "color")]
    pub skip_colors: bool,

    pub ui_handle: Option<ui::state::Handle>,
}

impl Player {

    fn set_current(&self, info: tracks::Info) {
        let arc_info = Arc::new(info);
        self.current.store(Some(arc_info.clone()));
        self.notify_ui_track(ui::state::Current::Track(arc_info));
    }

    #[cfg(feature = "color")]
    pub async fn get_color_palette(&self, info: &tracks::Info) -> Option<Vec<[u8; 3]>> {
        self.art_cache.get_color_palette(info).await
    }

    #[cfg(feature = "color")]
    pub async fn get_art(&self, info: &tracks::Info) -> Option<Vec<u8>> {
        self.art_cache.get_art(info).await
    }

    #[cfg(feature = "color")]
    pub async fn update_current_with_colors(&self) {
        if let Some(current) = self.current.load().as_ref() {
            if let Some(updated) = self.art_cache.update_current_with_colors(current).await {
                self.current.store(Some(updated.clone()));
                self.notify_ui_track(ui::state::Current::Track(updated));
            }
        }
    }

    pub fn current_exists(&self) -> bool {
        self.current.load().is_some()
    }

    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }

    pub fn notify_ui_track(&self, current: ui::state::Current) {
        if let Some(ref handle) = self.ui_handle {
            let _ = handle.update(ui::state::Update::Track(current));
        }
    }

    pub fn notify_ui_volume(&self) {
        if let Some(ref handle) = self.ui_handle {
            let _ = handle.update(ui::state::Update::Volume);
        }
    }

    pub fn notify_ui_bookmark(&self, bookmarked: bool) {
        if let Some(ref handle) = self.ui_handle {
            let _ = handle.update(ui::state::Update::Bookmarked(bookmarked));
        }
    }

    pub fn notify_ui_quit(&self) {
        if let Some(ref handle) = self.ui_handle {
            let _ = handle.update(ui::state::Update::Quit);
        }
    }

    pub fn get_current_track_data(&self) -> Option<Bytes> {
        let current = self.current.load();
        current
            .as_ref()
            .and_then(|info| info.raw_data.as_ref())
            .map(|data| (**data).clone())
    }

    pub fn get_bandcamp_client(&self) -> eyre::Result<reqwest::Client> {
        crate::bandcamp::discography::DiscographyParser::create_http_client()
    }

    pub async fn new(args: &Args) -> eyre::Result<(Self, OutputStream), player::Error> {
        debug_log!("player.rs - new: initialization start buffer_size={} timeout={} paused={} debug={}", args.buffer_size, args.timeout, args.paused, args.debug);

        let bookmarks = Bookmarks::load().await?;

        let volume = PersistentVolume::load()
            .await
            .map_err(player::Error::PersistentVolumeLoad)?;

            let list = List::load(args.track_list.as_ref(),
            #[cfg(feature = "bandcamp")]
            !args.archive,
            #[cfg(not(feature = "bandcamp"))]
            false
        )
            .await
            .map_err(player::Error::TrackListLoad)?;

        #[cfg(target_os = "linux")]
        let mut stream = if !args.alternate && !args.debug {
            audio::silent_get_output_stream()?
        } else {
            OutputStreamBuilder::open_default_stream()?
        };

        #[cfg(not(target_os = "linux"))]
        let mut stream = OutputStreamBuilder::open_default_stream()?;

        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());

        if args.paused {
            sink.pause();
        }

        let client = Client::builder()
            .user_agent(concat!(
                env!("CARGO_PKG_NAME"),
                "/",
                env!("CARGO_PKG_VERSION")
            ))

            .timeout(Duration::from_secs(10))
            .connect_timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .build()?;

        let player = Self {
            tracks: RwLock::new(VecDeque::with_capacity(args.buffer_size)),
            buffer_size: args.buffer_size,
            current: ArcSwapOption::new(None),
            progress: AtomicF32::new(0.0),
            timeout: Duration::from_secs(args.timeout),
            bookmarks,
            client,
            sink: Arc::new(sink),
            volume,
            list,
            #[cfg(feature = "color")]
            art_cache: Arc::new(ui::cover::ArtCache::new()),
            skip_art: {
                #[cfg(feature = "color")]
                { args.colorless && args.art.is_none() }
                #[cfg(not(feature = "color"))]
                { true }
            },
            #[cfg(feature = "color")]
            skip_colors: {
                #[cfg(feature = "color")]
                { args.colorless }
                #[cfg(not(feature = "color"))]
                { true }
            },
            ui_handle: None,
        };
        debug_log!("player.rs - new: initialization completed");
        Ok((player, stream))
    }

    pub async fn play(
        player: Arc<Self>,
        tx: Sender<Message>,
        mut rx: Receiver<Message>,
        debug: bool,
    ) -> eyre::Result<(), player::Error> {
        debug_log!("player.rs - play: playback loop start");

        #[cfg(feature = "mpris")]
        let mpris = mpris::Server::new(Arc::clone(&player), tx.clone())
            .await
            .inspect_err(|x| {
                debug_log!("player.rs - play: initialization error: {:?}", x);
            })?;

        let downloader = Downloader::new(Arc::clone(&player));
        let (itx, downloader) = downloader.start(debug);

        Downloader::notify(&itx).await?;

        player.set_volume(player.volume.float());

        let mut new = false;

        loop {
            let clone = Arc::clone(&player);

            let msg = select! {
                biased;

                Some(x) = rx.recv() => x,

                Ok(()) = task::spawn_blocking(move || clone.sink.sleep_until_end()),
                        if new => Message::Next,
            };
            debug_log!("player.rs - play: message received: {:?}", msg);

            match msg {
                Message::Next | Message::Init | Message::TryAgain => {

                    new = false;

                    if msg == Message::Next && !player.current_exists() {
                        continue;
                    }

                    task::spawn(Self::next(
                        Arc::clone(&player),
                        itx.clone(),
                        tx.clone(),
                        debug,
                    ));
                }
                Message::Play => {
                    player.sink.play();
                    debug_log!("player.rs - play: playback started");

                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Playing).await?;
                }
                Message::Pause => {
                    player.sink.pause();
                    debug_log!("player.rs - play: playback paused");

                    #[cfg(feature = "mpris")]
                    mpris.playback(PlaybackStatus::Paused).await?;
                }
                Message::PlayPause => {
                    if player.sink.is_paused() {
                        player.sink.play();
                        debug_log!("player.rs - play: toggle play/pause -> play");
                    } else {
                        player.sink.pause();
                        debug_log!("player.rs - play: toggle play/pause -> pause");
                    }

                    #[cfg(feature = "mpris")]
                    mpris
                        .playback(mpris.player().playback_status().await?)
                        .await?;
                }
                Message::ChangeVolume(change) => {
                    player.set_volume(player.sink.volume() + change);
                    debug_log!("player.rs - play: volume changed to {}", player.sink.volume());

                    player.notify_ui_volume();

                    #[cfg(feature = "mpris")]
                    mpris
                        .changed(vec![Property::Volume(player.sink.volume().into())])
                        .await?;
                }

                Message::NewSong => {

                    new = true;
                    debug_log!("player.rs - play: new song started");
                    #[cfg(feature = "mpris")]
                    mpris
                        .changed(vec![
                            Property::Metadata(mpris.player().metadata().await?),
                            Property::PlaybackStatus(mpris.player().playback_status().await?),
                        ])
                        .await?;

                    continue;
                }
                Message::Bookmark => {
                    let current = player.current.load();
                    let current = current.as_ref().unwrap();

                    player.bookmarks.bookmark(current).await?;
                    debug_log!("player.rs - play: bookmark created for path={}", current.full_path);

                    player.notify_ui_bookmark(player.bookmarks.bookmarked());
                }
                Message::Quit => {

                    player.notify_ui_quit();
                    break;
                }
            }
        }

        downloader.abort();
        debug_log!("player.rs - play: playback loop exit");

        Ok(())
    }
}
