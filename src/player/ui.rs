#![allow(
    clippy::as_conversions,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "the ui is full of these because of various layout & positioning aspects, and for a simple music player making all casts safe is not worth the effort"
)]

use std::{
    fmt::Write as _,
    io::{stdout, Stdout},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::Args;

#[cfg(feature = "color")]
use crate::ArtStyle;

use crossterm::{
    cursor::{Hide, MoveTo, MoveToColumn, MoveUp, Show},
    event::{KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags},
    style::{Print, Stylize as _},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use lazy_static::lazy_static;
use thiserror::Error;
use tokio::{sync::{broadcast, mpsc::Sender}, task, time::{sleep, Instant}};
use unicode_segmentation::UnicodeSegmentation;

use super::Player;
use crate::messages::Message;

mod clock;
mod components;
mod input;

#[cfg(feature = "color")]
mod art;

pub mod cover;
pub mod state;

#[derive(Debug, Error)]
pub enum UIError {
    #[error("unable to convert number")]
    Conversion(#[from] std::num::TryFromIntError),

    #[error("unable to write output")]
    Write(#[from] std::io::Error),

    #[error("sending message to backend from ui failed")]
    Communication(#[from] tokio::sync::mpsc::error::SendError<Message>),
}

lazy_static! {

    static ref VOLUME_TIMER: AtomicUsize = AtomicUsize::new(0);
}

pub fn flash_audio() {
    VOLUME_TIMER.store(1, Ordering::Relaxed);
}

pub struct Window {

    borderless: bool,

    borders: [String; 2],

    width: usize,

    out: Stdout,
}

impl Window {

    pub fn new(width: usize, borderless: bool) -> Self {
        let borders = if borderless {
            [String::new(), String::new()]
        } else {
            let middle = "─".repeat(width + 2);

            [format!("┌{middle}┐"), format!("└{middle}┘")]
        };

        Self {
            borders,
            borderless,
            width,
            out: stdout(),
        }
    }

    pub fn draw(&mut self, content: Vec<String>, space: bool) -> eyre::Result<(), UIError> {
        let len: u16 = content.len().try_into()?;

        let menu: String = content.into_iter().fold(String::new(), |mut output, x| {

            let padding = if self.borderless { " " } else { "│" };
            let space = if space {
                " ".repeat(self.width.saturating_sub(x.graphemes(true).count()))
            } else {
                String::new()
            };
            write!(output, "{padding} {}{space} {padding}\r\n", x.reset()).unwrap();

            output
        });

        #[cfg(windows)]
        let (height, suffix) = (len + 2, "\r\n");
        #[cfg(not(windows))]
        let (height, suffix) = (len + 1, "");

        let rendered = format!("{}\r\n{menu}{}{suffix}", self.borders[0], self.borders[1]);

        crossterm::execute!(
            self.out,
            MoveToColumn(0),
            Print(&rendered),
            Clear(ClearType::FromCursorDown),
            MoveToColumn(0),
            MoveUp(height),
        )?;

        Ok(())
    }
}

#[cfg(feature = "color")]
type OptionalArtStyle = Option<ArtStyle>;
#[cfg(not(feature = "color"))]
type OptionalArtStyle = Option<()>;

async fn interface(
    player: Arc<Player>,
    mut receiver: broadcast::Receiver<state::Update>,
    minimalist: bool,
    borderless: bool,
    debug: bool,
    fps: u8,
    width: usize,
    colorize: bool,
    art_style: OptionalArtStyle,
    show_clock: bool,
) -> eyre::Result<(), UIError> {
    let mut window = Window::new(width, borderless || debug);
    let mut last_track_path: Option<String> = None;
    let mut clock = if show_clock { Some(clock::Clock::new()) } else { None };

    let mut state = state::State::initial(Arc::clone(&player.sink));

    #[cfg(feature = "color")]
    let mut cached_art: Option<art::AlbumCover> = None;

    loop {

        match receiver.try_recv() {
             Ok(update) => {
                 match update {
                     state::Update::Volume => {
                         state.volume_timer = Some(Instant::now());
                     }
                     state::Update::Quit => break,
                     state::Update::Track(current) => {
                         state.current = current;
                     }
                     state::Update::Bookmarked(b) => {
                         state.bookmarked = b;
                     }
                 }
             }
             Err(broadcast::error::TryRecvError::Closed) => break,
             Err(_) => {}
        }

        state.tick();

        if let state::Current::Loading(_) = state.current {
            state.current = state::Current::Loading(player.progress.load(Ordering::Relaxed));
        }

        #[cfg(feature = "color")]
        {
            player.update_current_with_colors().await;
        }

        #[cfg(feature = "color")]
        {
            if let Some(style) = &art_style {
                let current_track = state.current.track();
                let current_path = current_track.map(|c| c.full_path.as_str());
                let last_path = last_track_path.as_deref();

                if current_path != last_path {
                    let current_after_update = player.current.load();

                    cached_art = if let Some(current) = current_after_update.as_ref() {
                        if let Some(art_url) = &current.art_url {
                            if !art_url.is_empty() && art_url.starts_with("http") {
                                if let Some(cached_data) = player.get_art(current).await {
                                    art::AlbumCover::from_image_data(&cached_data, width, style, colorize)
                                } else {
                                    if let Ok(client) = player.get_bandcamp_client() {
                                        if let Some((_palette, image_data)) = cover::extract_color_palette_and_bytes_from_url_with_client(&client, art_url).await {
                                            art::AlbumCover::from_image_data(&image_data, width, style, colorize)
                                        } else {
                                            None
                                        }
                                    } else {
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                     if cached_art.is_none() && !player.skip_art {
                         if let Some(data) = player.get_current_track_data() {
                             cached_art = art::AlbumCover::from_track_data(&data, width, style, colorize);
                         } else {
                             cached_art = None;
                         }
                    }

                    last_track_path = current_path.map(ToString::to_string);
                }
            } else {
                cached_art = None;
                last_track_path = None;
            }
        }

        #[cfg(not(feature = "color"))]
        let _ = (art_style, &last_track_path);

        let action = components::action(&state, width, colorize);
        let volume = state.sink.volume();
        let percentage = format!("{}%", (volume * 100.0).round().abs());
        let palette = state.current.track().and_then(|c| c.color_palette.as_ref());

        let middle = if state.show_volume() {
             components::audio_bar(volume, &percentage, width - 17, palette, colorize)
        } else {
             components::progress_bar(&state, width - 16, colorize)
        };

        let controls = components::controls(width, palette, colorize);
        let mut menu = Vec::new();

        if let Some(ref mut clk) = clock {
            let time_str = clk.get_time();
            let clock_line = format!(" {} ", time_str);
            let padding = " ".repeat(width.saturating_sub(clock_line.len()));
            menu.push(format!("{}{}", clock_line, padding));
        }

        #[cfg(feature = "color")]
        if let Some(art) = &cached_art {
            menu.extend(art.lines.iter().cloned());
            if !art.lines.is_empty() {
                menu.push(" ".repeat(width));
            }
        }

        match (minimalist, debug, state.current.track()) {
            (true, _, _) => {
                menu.push(action);
                menu.push(middle);
            }
            (false, true, Some(_x)) => {
                menu.push(action);
                menu.push(middle);
                menu.push(controls);
            }
            _ => {
                menu.push(action);
                menu.push(middle);
                menu.push(controls);
            }
        }

        window.draw(menu, false)?;

        let delta = 1.0 / f32::from(fps);
        sleep(Duration::from_secs_f32(delta)).await;
    }

    Ok(())
}

pub struct Environment {

    enhancement: bool,

    alternate: bool,
}

impl Environment {

    pub fn ready(alternate: bool) -> eyre::Result<Self, UIError> {
        let mut lock = stdout().lock();

        crossterm::execute!(lock, Hide)?;

        if alternate {
            crossterm::execute!(lock, EnterAlternateScreen, MoveTo(0, 0))?;
        }

        terminal::enable_raw_mode()?;
        let enhancement = terminal::supports_keyboard_enhancement()?;

        if enhancement {
            crossterm::execute!(
                lock,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            )?;
        }

        Ok(Self {
            enhancement,
            alternate,
        })
    }

    pub fn cleanup(&self) -> eyre::Result<(), UIError> {
        let mut lock = stdout().lock();

        if self.alternate {
            crossterm::execute!(lock, LeaveAlternateScreen)?;
        }

        crossterm::execute!(lock, Clear(ClearType::FromCursorDown), Show)?;

        if self.enhancement {
            crossterm::execute!(lock, PopKeyboardEnhancementFlags)?;
        }

        terminal::disable_raw_mode()?;

        eprintln!("bye! ₍^ >ヮ<^₎ .ᐟ.ᐟ ₍^._.^₎𐒡 ₍^.^₎⊃ヾ(≧▽≦*)ゝ =^•⩊•^=  >ܫ< ૮꒰ ˶• ༝ •˶꒱ა ♡ /ᐠ - ˕ -マ Nyaa...");

        Ok(())
    }
}

impl Drop for Environment {

    fn drop(&mut self) {

        let _ = self.cleanup();
    }
}

pub async fn start(
    player: Arc<Player>,
    sender: Sender<Message>,
    receiver: broadcast::Receiver<state::Update>,
    args: Args,
) -> eyre::Result<(), UIError> {
    let environment = Environment::ready(args.alternate)?;

    #[cfg(feature = "color")]
    let (colorize, art) = (!args.colorless, args.art);
    #[cfg(not(feature = "color"))]
    let (colorize, art): (bool, OptionalArtStyle) = (false, None);

    let total_width = 22 + args.width.min(32) * 2;

    let interface = task::spawn(interface(
        Arc::clone(&player),
        receiver,
        args.minimalist,
        args.borderless,
        args.debug,
        args.fps,
        total_width,
        colorize,
        art,
        args.clock,
    ));

    input::listen(sender.clone()).await?;
    interface.abort();

    environment.cleanup()?;

    Ok(())
}
