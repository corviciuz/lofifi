#![allow(dead_code)]

use std::future::Future;
use tokio::{select, sync::mpsc, task::JoinSet};

use crate::messages::Message;

pub struct Tasks {

    pub set: JoinSet<Result<(), TaskError>>,

    tx: mpsc::Sender<Message>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("message send error: {0}")]
    Send(#[from] mpsc::error::SendError<Message>),
}

impl Tasks {

    pub fn new(tx: mpsc::Sender<Message>) -> Self {
        Self {
            tx,
            set: JoinSet::new(),
        }
    }

    pub fn spawn<F>(&mut self, future: F)
    where
        F: Future<Output = Result<(), TaskError>> + Send + 'static,
    {
        self.set.spawn(future);
    }

    pub fn tx(&self) -> mpsc::Sender<Message> {
        self.tx.clone()
    }

    pub async fn wait<F, E>(&mut self, runner: F) -> Result<(), TaskError>
    where
        F: Future<Output = Result<(), E>> + Send,
        E: std::fmt::Debug,
    {
        select! {
            result = runner => {
                if let Err(e) = result {

                    log::warn!("Runner completed with error: {:?}", e);
                }
                Ok(())
            },
            Some(result) = self.set.join_next() => {
                match result {
                    Ok(res) => res,
                    Err(e) if !e.is_cancelled() => Err(TaskError::Join(e)),
                    Err(_) => Ok(()),
                }
            }
        }
    }

    pub fn abort_all(&mut self) {
        self.set.abort_all();
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        self.abort_all();
    }
}
