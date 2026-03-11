use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use tokio::sync::RwLock;
use tokio::{fs, io};

use crate::{data_dir, tracks, debug_log};

#[derive(Debug, thiserror::Error)]
pub enum BookmarkError {
    #[error("data directory not found")]
    DataDir,

    #[error("io failure")]
    Io(#[from] io::Error),
}

pub struct Bookmarks {

    entries: RwLock<Vec<String>>,

    bookmarked: AtomicBool,
}

impl Bookmarks {
    fn key_of(entry: &str) -> &str {
        entry.split('!').next().unwrap_or(entry)
    }

    pub async fn path() -> eyre::Result<PathBuf, BookmarkError> {
        debug_log!("bookmark.rs - path: getting bookmarks file path");
        let data_dir = data_dir().map_err(|_| BookmarkError::DataDir)?;
        debug_log!("bookmark.rs - path: creating data directory: {}", data_dir.display());
        fs::create_dir_all(data_dir.clone()).await?;

        let bookmarks_path = data_dir.join("bookmarks.txt");
        debug_log!("bookmark.rs - path: bookmarks file path: {}", bookmarks_path.display());
        Ok(bookmarks_path)
    }

    pub async fn load() -> eyre::Result<Self, BookmarkError> {
        debug_log!("bookmark.rs - load: loading bookmarks");
        let bookmarks_path = Self::path().await?;
        let text = fs::read_to_string(bookmarks_path)
            .await
            .unwrap_or_default();

        let lines: Vec<String> = text
            .trim_start_matches("noheader")
            .trim()
            .lines()
            .filter_map(|x| {
                if x.is_empty() {
                    None
                } else {
                    Some(x.to_string())
                }
            })
            .collect();

        debug_log!("bookmark.rs - load: loaded {} bookmarks", lines.len());
        Ok(Self {
            entries: RwLock::new(lines),
            bookmarked: AtomicBool::new(false),
        })
    }

    pub async fn save(&self) -> eyre::Result<(), BookmarkError> {
        debug_log!("bookmark.rs - save: saving bookmarks");
        let bookmarks_path = Self::path().await?;
        let entries = self.entries.read().await;
        debug_log!("bookmark.rs - save: saving {} bookmarks to: {}", entries.len(), bookmarks_path.display());
        let text = format!("noheader\n{}", entries.join("\n"));
        fs::write(bookmarks_path, text).await?;
        Ok(())
    }

    pub async fn bookmark(&self, track: &tracks::Info) -> eyre::Result<(), BookmarkError> {
        debug_log!("bookmark.rs - bookmark: toggling bookmark for track: {}", track.display_name);
        let entry = track.to_entry();
        let entry_key = Self::key_of(&entry).to_string();
        let mut entries = self.entries.write().await;
        let idx = entries.iter().position(|x| Self::key_of(x) == entry_key);

        if let Some(idx) = idx {
            debug_log!("bookmark.rs - bookmark: removing bookmark for track: {}", track.display_name);
            entries.remove(idx);
        } else {
            debug_log!("bookmark.rs - bookmark: adding bookmark for track: {}", track.display_name);
            entries.push(entry);
        };

        let is_now_bookmarked = entries.iter().any(|x| Self::key_of(x) == entry_key);
        self.bookmarked
            .swap(is_now_bookmarked, std::sync::atomic::Ordering::Relaxed);

        debug_log!("bookmark.rs - bookmark: track {} is now bookmarked: {}", track.display_name, is_now_bookmarked);
        Ok(())
    }

    pub fn bookmarked(&self) -> bool {
        self.bookmarked.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub async fn set_bookmarked(&self, track: &tracks::Info) {
        debug_log!("bookmark.rs - set_bookmarked: checking bookmark status for track: {}", track.display_name);
        let key = Self::key_of(&track.full_path);
        let val = self.entries.read().await.iter().any(|x| Self::key_of(x) == key);
        self.bookmarked
            .swap(val, std::sync::atomic::Ordering::Relaxed);
        debug_log!("bookmark.rs - set_bookmarked: track {} bookmark status: {}", track.display_name, val);
    }
}
