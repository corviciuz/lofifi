#![warn(clippy::all, clippy::pedantic, clippy::nursery)]

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

mod messages;
mod play;
mod player;
mod tracks;
mod dbg;
mod tasks;
mod bandcamp {
    pub mod discography;
    pub use discography::*;
    pub mod check;
}

#[allow(clippy::all, clippy::pedantic, clippy::nursery, clippy::restriction)]
#[cfg(feature = "scrape")]
mod scrapers;

#[cfg(feature = "scrape")]
use crate::scrapers::Source;

#[cfg(feature = "color")]
#[derive(ValueEnum, Clone, Debug)]
#[clap(rename_all = "kebab-case")]
pub enum ArtStyle {
    Pixel,
    #[clap(name = "ascii-bg")]
    AsciiBg,
    Ascii,
}

#[derive(Parser, Clone)]
#[command(about, version)]
#[allow(clippy::struct_excessive_bools)]
pub struct Args {

    #[clap(long, short)]
    alternate: bool,

    #[clap(long, short)]
    minimalist: bool,

    #[clap(long, short)]
    borderless: bool,

    #[clap(long, short)]
    clock: bool,

    #[clap(long, short)]
    paused: bool,

    #[clap(long, short, default_value_t = 12)]
    fps: u8,

    #[clap(long, default_value_t = 3)]
    timeout: u64,

    #[clap(long, short)]
    debug: bool,

    #[clap(long, short, default_value_t = 16)]
    width: usize,

    #[cfg(feature = "color")]
    #[clap(long)]
    art: Option<ArtStyle>,

    #[cfg(feature = "color")]
    #[clap(long)]
    colorless: bool,

    #[clap(long, short, alias = "list", alias = "tracks", short_alias = 'l')]
    track_list: Option<String>,

    #[clap(long)]
    archive: bool,

    #[clap(long, short = 's', alias = "buffer", default_value_t = 10)]
    buffer_size: usize,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Clone, Debug)]
enum Commands {

    #[cfg(feature = "scrape")]
    Scrape {

        source: scrapers::Source,
    },

    Check {

        #[clap(long)]
        out: PathBuf,

        #[clap(long)]
        url: Option<String>,

        #[clap(long)]
        lyrics: bool,

        #[clap(long)]
        name: Option<String>,

        #[clap(long)]
        pl: bool,

        #[clap(long)]
        nodl: bool,

        #[clap(long)]
        complete: bool,
    },

    #[cfg(feature = "presave")]
    PresaveBandcamp {
        url: String,
        #[clap(long, default_value_t = 0)]
        max_albums: usize,
    },
}

pub fn data_dir() -> eyre::Result<PathBuf, player::Error> {
    let dir = dirs::data_dir()
        .ok_or(player::Error::DataDir)?
        .join("lofifi");

    Ok(dir)
}

pub fn cache_dir() -> eyre::Result<PathBuf, player::Error> {
    let dir = data_dir()?.join("cache");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).map_err(|_| player::Error::DataDir)?;
    }
    Ok(dir)
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    debug_log!("main.rs - main: starting lowfi application");
    color_eyre::install()?;

    debug_log!("main.rs - main: parsing command line arguments");
    let cli = Args::parse();

    if cli.debug {
        debug_log!("main.rs - main: debug mode enabled, initializing logger");

        let mut builder = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug,html5ever=warn,selectors=warn"));
        builder.format(|buf, record| {
            use std::io::Write;
            let level = record.level();
            let mut msg = record.args().to_string();
            while msg.ends_with('\n') { msg.pop(); }
            writeln!(buf, "{}: {}", level, msg)
        }).init();
        dbg::enable();
        debug_log!("main.rs - main: logger initialized and debug logging enabled");
    }

    if let Some(command) = cli.command {
        debug_log!("main.rs - main: executing command: {:?}", command);
        match command {
            #[cfg(feature = "scrape")]
            Commands::Scrape { source } => {
                debug_log!("main.rs - main: executing scrape command for source: {:?}", source);
                match source {
                    Source::Archive => scrapers::archive::scrape().await?,
                    Source::Lofigirl => scrapers::lofigirl::scrape().await?,
                    Source::Chillhop => scrapers::chillhop::scrape().await?,
                }
            },
            #[cfg(feature = "presave")]
            Commands::PresaveBandcamp { url, max_albums } => {
                debug_log!("main.rs - main: executing presave command for URL: {} max_albums: {:?}", url, max_albums);
                tracks::presave::create_presaved_bandcamp_list(&url, max_albums).await?;
            },
            Commands::Check { out, url, lyrics, name, pl, nodl, complete } => {
                let base = url.unwrap_or_else(|| bandcamp::check::default_base_url());
                debug_log!("main.rs - main: executing check command base_url={} out=\"{}\"", base, out.display());
                bandcamp::check::run(&out, &base, cli.timeout, cli.debug, lyrics, name, pl, nodl, complete).await?;
            },
        }
    } else {
        debug_log!("main.rs - main: no command specified, starting audio player");
        play::play(cli).await?;
    };

    debug_log!("main.rs - main: application completed successfully");
    Ok(())
}
