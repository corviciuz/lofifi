use std::ffi::NulError;

use crate::{messages::Message, player::bookmark::BookmarkError};
use tokio::sync::mpsc::error::SendError;

#[cfg(feature = "mpris")]
use mpris_server::zbus::{self, fdo};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("unable to load the persistent volume")]
    PersistentVolumeLoad(eyre::Error),

    #[error("unable to save the persistent volume")]
    PersistentVolumeSave(eyre::Error),

    #[error("sending internal message failed")]
    Communication(#[from] SendError<Message>),

    #[error("unable to load track list")]
    TrackListLoad(eyre::Error),

    #[error("interfacing with audio failed")]
    Stream(#[from] rodio::StreamError),

    #[error("NUL error, if you see this, something has gone VERY wrong")]
    Nul(#[from] NulError),

    #[error("unable to send or prepare network request")]
    Reqwest(#[from] reqwest::Error),

    #[cfg(feature = "mpris")]
    #[error("mpris bus error")]
    ZBus(#[from] zbus::Error),

    #[cfg(feature = "mpris")]
    #[error("mpris fdo (zbus interface) error")]
    Fdo(#[from] fdo::Error),

    #[error("unable to notify downloader")]
    DownloaderNotify(#[from] SendError<()>),

    #[error("unable to find data directory")]
    DataDir,

    #[error("bookmarking load/unload failed")]
    Bookmark(#[from] BookmarkError),

    #[error("ui error: {0}")]
    UI(#[from] crate::player::ui::UIError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("task join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
}
