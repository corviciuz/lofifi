use std::sync::{atomic::Ordering, Arc};
use tokio::{sync::mpsc::Sender, time::sleep};

use crate::{
    messages::Message,
    player::{downloader::Downloader, Player},
    tracks,
};
use crate::debug_log;

impl Player {

    async fn fetch(&self) -> Result<tracks::DecodedTrack, tracks::Error> {
        debug_log!("queue.rs - fetch: fetch start");

        let track = self.tracks.write().await.pop_front();
        let track = if let Some(track) = track {
            debug_log!("queue.rs - fetch: popped from buffer full_path={}", track.full_path);
            track
        } else {

            self.current.store(None);
            self.progress.store(0.0, Ordering::Relaxed);
            debug_log!("queue.rs - fetch: buffer empty; fetching random");

            let (path, custom_name, art_url) = self.list.random_path()
                .ok_or_else(|| tracks::Error {
                    track: "list".to_string(),
                    kind: tracks::error::Kind::EmptyList,
                })?;

            #[cfg(feature = "color")]
            let cover_future = {
                let client = self.client.clone();
                let art_opt = art_url.clone();
                let skip_art = self.skip_art;
                async move {
                    if !skip_art {
                        if let Some(url) = art_opt {
                            if !url.is_empty() && url.starts_with("http") {
                                crate::player::ui::cover::extract_color_palette_and_bytes_from_url_with_client(&client, &url).await
                            } else { None }
                        } else { None }
                    } else { None }
                }
            };

            let download_future = self.list.download(&path, &self.client, Some(&self.progress));

            #[cfg(feature = "color")]
            let (cover_opt, download_res) = tokio::join!(cover_future, download_future);
            #[cfg(feature = "color")]
            let (data, full_path) = download_res?;
            #[cfg(not(feature = "color"))]
            let (data, full_path) = download_future.await?;

            #[cfg(feature = "color")]
            if let Some((palette, image_data)) = cover_opt {
                if let Some(url) = &art_url {
                    self.art_cache.cache_art(url.clone(), image_data).await;
                    if !self.skip_colors {
                        self.art_cache.cache_colors(url.clone(), palette).await;
                    }
                }
            }

            let name = custom_name.map_or_else(
                || crate::tracks::TrackName::Raw(path.clone()),
                crate::tracks::TrackName::Formatted,
            );

            crate::tracks::QueuedTrack { name, full_path, data, art_url }
        };

        let decoded = track.decode()?;
        debug_log!("queue.rs - fetch: decoded display_name={} duration={:?}", decoded.info.display_name, decoded.info.duration);

        #[cfg(feature = "color")]
        let final_info = {
            let mut info = decoded.info.clone();
            if info.color_palette.is_none() {
                if let Some(palette) = self.get_color_palette(&info).await {
                    info.color_palette = Some(palette);
                }
            }

            if let Some(art_url) = &info.art_url {
                if !art_url.is_empty() && art_url.starts_with("http") && !self.skip_art {
                    if self.get_art(&info).await.is_none() {
                        if let Some((palette, image_data)) = crate::player::ui::cover::extract_color_palette_and_bytes_from_url_with_client(&self.client, art_url).await {
                            if let Some(art_url) = &info.art_url {
                                self.art_cache.cache_art(art_url.clone(), image_data).await;

                                if !self.skip_colors {
                                    self.art_cache.cache_colors(art_url.clone(), palette).await;
                                }
                            }
                        }
                    }
                }
            }
            info
        };

        #[cfg(not(feature = "color"))]
        let final_info = decoded.info.clone();

        self.set_current(final_info);
        debug_log!("queue.rs - fetch: current track set");

        Ok(decoded)
    }

    pub async fn next(
        player: Arc<Self>,
        itx: Sender<()>,
        tx: Sender<Message>,
        _debug: bool,
    ) -> eyre::Result<()> {

        player.sink.stop();

        let track = player.fetch().await;

        match track {
            Ok(track) => {

                player.sink.append(track.data);
                debug_log!("queue.rs - next: track appended to sink");

                player.bookmarks.set_bookmarked(&track.info).await;

                Downloader::notify(&itx).await?;
                debug_log!("queue.rs - next: downloader notified");

                tx.send(Message::NewSong).await?;
                debug_log!("queue.rs - next: NewSong message sent");
            }
            Err(error) => {
                debug_log!("queue.rs - next: error occurred err={}", error);

                if !error.is_timeout() {
                    sleep(player.timeout).await;
                }

                tx.send(Message::TryAgain).await?;
            }
        };

        Ok(())
    }
}
