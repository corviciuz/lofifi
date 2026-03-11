use std::sync::Arc;
use tokio::{sync::broadcast, time::Instant};

use crate::tracks::Info;

#[derive(Clone, Debug)]
pub enum Current {

    Loading(f32),

    Track(Arc<Info>),
}

impl Default for Current {
    fn default() -> Self {
        Self::Loading(0.0)
    }
}

impl Current {

    pub fn track(&self) -> Option<&Arc<Info>> {
        match self {
            Self::Track(info) => Some(info),
            Self::Loading(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct State {

    pub sink: Arc<rodio::Sink>,

    pub current: Current,

    pub bookmarked: bool,

    pub volume_timer: Option<Instant>,
}

impl State {

    pub fn initial(sink: Arc<rodio::Sink>) -> Self {
        Self {
            sink,
            current: Current::default(),
            bookmarked: false,
            volume_timer: None,
        }
    }

    pub fn tick(&mut self) {
        let expired = |timer: Instant| timer.elapsed() > std::time::Duration::from_secs(1);
        if self.volume_timer.is_some_and(expired) {
            self.volume_timer = None;
        }
    }

    pub fn show_volume(&self) -> bool {
        self.volume_timer.is_some()
    }
}

#[derive(Debug, Clone)]
pub enum Update {

    Track(Current),

    Bookmarked(bool),

    Volume,

    Quit,
}

pub struct Handle {

    sender: broadcast::Sender<Update>,
}

impl Handle {

    pub fn new(sender: broadcast::Sender<Update>) -> Self {
        Self { sender }
    }

    pub fn update(&self, update: Update) -> Result<(), broadcast::error::SendError<Update>> {
        self.sender.send(update)?;
        Ok(())
    }
}

pub fn channel() -> (Handle, broadcast::Receiver<Update>) {
    let (sender, receiver) = broadcast::channel(16);
    (Handle::new(sender), receiver)
}
