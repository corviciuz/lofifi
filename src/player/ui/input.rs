use crossterm::event::{self, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use futures::{FutureExt as _, StreamExt as _};
use tokio::sync::mpsc::Sender;

use crate::player::{
    ui::{self, UIError},
    Message,
};

pub async fn listen(sender: Sender<Message>) -> eyre::Result<(), UIError> {
    let mut reader = EventStream::new();

    loop {
        let Some(Ok(event::Event::Key(event))) = reader.next().fuse().await else {
            continue;
        };

        if event.kind == KeyEventKind::Release {
            continue;
        }

        let messages = match event.code {

            KeyCode::Up => Message::ChangeVolume(0.1),
            KeyCode::Right => Message::ChangeVolume(0.01),
            KeyCode::Down => Message::ChangeVolume(-0.1),
            KeyCode::Left => Message::ChangeVolume(-0.01),
            KeyCode::Char(character) => match character.to_ascii_lowercase() {

                'c' if event.modifiers == KeyModifiers::CONTROL => Message::Quit,

                'q' => Message::Quit,

                's' | 'n' | 'l' => Message::Next,

                'p' | ' ' => Message::PlayPause,

                '+' | '=' | 'k' => Message::ChangeVolume(0.1),
                '-' | '_' | 'j' => Message::ChangeVolume(-0.1),

                'b' => Message::Bookmark,

                _ => continue,
            },

            KeyCode::Media(media) => match media {
                event::MediaKeyCode::Pause
                | event::MediaKeyCode::Play
                | event::MediaKeyCode::PlayPause => Message::PlayPause,
                event::MediaKeyCode::Stop => Message::Pause,
                event::MediaKeyCode::TrackNext => Message::Next,
                event::MediaKeyCode::LowerVolume => Message::ChangeVolume(-0.1),
                event::MediaKeyCode::RaiseVolume => Message::ChangeVolume(0.1),
                event::MediaKeyCode::MuteVolume => Message::ChangeVolume(-1.0),
                _ => continue,
            },
            _ => continue,
        };

        if let Message::ChangeVolume(_) = messages {
            ui::flash_audio();
        }

        sender.send(messages).await?;
    }
}
