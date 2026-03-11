use std::sync::Arc;

use tokio::{
    sync::mpsc::{self, Receiver, Sender},
    task::{self, JoinHandle},
    time::sleep,
};

use super::Player;
use crate::debug_log;

pub struct Downloader {

    player: Arc<Player>,

    rx: Receiver<()>,

    tx: Sender<()>,
}

impl Downloader {

    pub async fn notify(sender: &Sender<()>) -> Result<(), mpsc::error::SendError<()>> {
        sender.send(()).await
    }

    pub fn new(player: Arc<Player>) -> Self {
        let (tx, rx) = mpsc::channel(8);
        Self { player, rx, tx }
    }

    pub async fn push_buffer(&self, _debug: bool) {
        debug_log!("downloader.rs - push_buffer: requesting random track");
        let data = self.player.list.random(&self.player.client, None).await;
        match data {
            Ok(track) => {
                debug_log!("downloader.rs - push_buffer: track received full_path={}", track.full_path);

                #[cfg(feature = "color")]
                if !self.player.skip_art {
                    if let Some(url) = &track.art_url {
                        if !url.is_empty() && url.starts_with("http") {
                            let client = self.player.client.clone();
                            let art_url = url.clone();
                            let art_cache = self.player.art_cache.clone();
                            let skip_colors = self.player.skip_colors;
                            tokio::spawn(async move {
                                if let Some((palette, image_data)) = crate::player::ui::cover::extract_color_palette_and_bytes_from_url_with_client(&client, &art_url).await {
                                    art_cache.cache_art(art_url.clone(), image_data).await;
                                    if !skip_colors {
                                        art_cache.cache_colors(art_url.clone(), palette).await;
                                    }
                                }
                            });
                        }
                    }
                }

                self.player.tracks.write().await.push_back(track);
                debug_log!("downloader.rs - push_buffer: pushed to queue size={}", self.player.tracks.read().await.len());
            }
            Err(error) => {
                debug_log!("downloader.rs - push_buffer: error fetching track err={}", error);

                if !error.is_timeout() {
                    sleep(self.player.timeout).await;
                }
            }
        }
    }

    pub fn start(mut self, debug: bool) -> (Sender<()>, JoinHandle<()>) {
        let tx = self.tx.clone();

        let handle = task::spawn(async move {

            while self.rx.recv().await == Some(()) {
                debug_log!("downloader.rs - start: notified to fill buffer");

                while self.player.tracks.read().await.len() < self.player.buffer_size {
                    self.push_buffer(debug).await;
                }
                debug_log!("downloader.rs - start: buffer filled size={}", self.player.tracks.read().await.len());
            }
        });

        (tx, handle)
    }
}
