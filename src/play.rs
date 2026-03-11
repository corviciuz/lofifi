use crossterm::cursor::Show;
use crossterm::event::PopKeyboardEnhancementFlags;
use crossterm::terminal::{self, Clear, ClearType};
use std::io::{stdout, IsTerminal};
use std::process::exit;
use std::sync::Arc;
use std::{env, panic};
use tokio::{sync::mpsc, task};

use crate::messages::Message;
use crate::player::persistent_volume::PersistentVolume;
use crate::player::Player;
use crate::player::{self, ui};
use crate::Args;

pub async fn play(args: Args) -> eyre::Result<(), player::Error> {

    let eyre_hook = panic::take_hook();

    panic::set_hook(Box::new(move |x| {
        let mut lock = stdout().lock();
        crossterm::execute!(
            lock,
            Clear(ClearType::FromCursorDown),
            Show,
            PopKeyboardEnhancementFlags
        )
        .unwrap();
        terminal::disable_raw_mode().unwrap();

        eyre_hook(x);
        exit(1)
    }));

    let (mut player, stream) = Player::new(&args).await?;

    let (handle, receiver) = ui::state::channel();
    player.ui_handle = Some(handle);

    let player = Arc::new(player);

    let (tx, rx) = mpsc::channel(8);
    let ui = if stdout().is_terminal() && !(env::var("LOWFI_DISABLE_UI") == Ok("1".to_owned())) {
        Some(task::spawn(ui::start(
            Arc::clone(&player),
            tx.clone(),
            receiver,
            args.clone(),
        )))
    } else {
        None
    };

    tx.send(Message::Init).await?;

    Player::play(Arc::clone(&player), tx.clone(), rx, args.debug).await?;

    PersistentVolume::save(player.sink.volume())
        .await
        .map_err(player::Error::PersistentVolumeSave)?;

    player.bookmarks.save().await?;

    drop(stream);
    player.sink.stop();
    if let Some(x) = ui {
        x.abort();
    }

    Ok(())
}
