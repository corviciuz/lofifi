use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use bytes::Bytes;
use convert_case::{Case, Casing};
use regex::Regex;
use rodio::{Decoder, Source as _};
use unicode_segmentation::UnicodeSegmentation;
use url::form_urlencoded;
use lazy_static::lazy_static;
use lofty::{file::TaggedFileExt, prelude::*, probe::Probe};

pub mod error;
pub mod list;
pub mod cache;
pub mod utils;

#[cfg(feature = "presave")]
pub mod presave;

pub use error::Error;

use crate::tracks::error::Context;
use crate::debug_log;

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek + ?Sized> ReadSeek for T {}

pub type DecodedData = Decoder<Box<dyn ReadSeek + Send + Sync>>;

#[derive(Clone, Default)]
pub struct SharedAudioBuffer(Arc<SharedAudioBufferInner>);

#[derive(Default)]
struct SharedAudioBufferInner {
    data: Mutex<Vec<u8>>,
    ready: Condvar,
    complete: AtomicBool,
}

impl SharedAudioBuffer {
    pub fn new() -> Self { Self::default() }

    pub fn append(&self, chunk: &[u8]) {
        let mut guard = self.0.data.lock().unwrap();
        guard.extend_from_slice(chunk);
        self.0.ready.notify_all();
    }

    pub fn mark_complete(&self) {
        self.0.complete.store(true, AtomicOrdering::Release);
        self.0.ready.notify_all();
    }

    pub fn snapshot(&self, max_bytes: usize) -> Bytes {
        let guard = self.0.data.lock().unwrap();
        let take = guard.len().min(max_bytes);
        if take == 0 {
            Bytes::new()
        } else {
            Bytes::copy_from_slice(&guard[..take])
        }
    }

    fn read_exact_range_blocking(&self, start: usize, len: usize, out: &mut [u8]) -> std::io::Result<usize> {
        let mut read_total = 0;
        let mut start_idx = start;
        while read_total < len {
            let mut guard = self.0.data.lock().unwrap();

            while guard.len() < start_idx + (len - read_total) && !self.0.complete.load(AtomicOrdering::Acquire) {
                guard = self.0.ready.wait(guard).unwrap();
            }

            let available = guard.len().saturating_sub(start_idx);
            if available == 0 {

                if self.0.complete.load(AtomicOrdering::Acquire) {
                    return Ok(read_total);
                }
                continue;
            }

            let to_copy = available.min(len - read_total);
            out[read_total..read_total + to_copy]
                .copy_from_slice(&guard[start_idx..start_idx + to_copy]);
            read_total += to_copy;
            start_idx += to_copy;
        }
        Ok(read_total)
    }
}

pub struct GrowingReader {
    buffer: SharedAudioBuffer,
    position: usize,
}

impl GrowingReader {
    pub fn new(buffer: SharedAudioBuffer) -> Self { Self { buffer, position: 0 } }
}

impl Read for GrowingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.buffer.read_exact_range_blocking(self.position, buf.len(), buf)?;
        self.position += read;
        Ok(read)
    }
}

impl Seek for GrowingReader {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        let new_pos: i128 = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.position as i128 + n as i128,
            SeekFrom::End(_n) => {

                return Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "SeekFrom::End not supported for streaming buffer"));
            }
        };

        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "negative seek"));
        }

        self.position = new_pos as usize;
        Ok(self.position as u64)
    }
}

#[derive(Debug, Clone)]
pub enum TrackName {

    Raw(String),

    Formatted(String),
}

#[derive(Clone)]
pub struct QueuedTrack {

    pub name: TrackName,

    pub full_path: String,

    pub data: TrackData,

    pub art_url: Option<String>,
}

#[derive(Clone)]
pub enum TrackData {
    Full(Bytes),
    Streaming(SharedAudioBuffer),
}

impl QueuedTrack {

    pub fn decode(self) -> eyre::Result<DecodedTrack, Error> {
        DecodedTrack::new(self)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub title: Option<String>,
    pub artist: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Info {

    pub full_path: String,

    pub custom_name: bool,

    pub display_name: String,

    pub width: usize,

    pub duration: Option<Duration>,

    pub metadata: Metadata,

    pub color_palette: Option<Vec<[u8; 3]>>,

    pub art_url: Option<String>,

    pub raw_data: Option<Arc<Bytes>>,
}

impl PartialEq for Info {
    fn eq(&self, other: &Self) -> bool {
        self.full_path == other.full_path
            && self.custom_name == other.custom_name
            && self.display_name == other.display_name
            && self.duration == other.duration
    }
}

impl Eq for Info {}

lazy_static! {
    static ref MASTER_PATTERNS: [Regex; 5] = [

        Regex::new(r"\s*\(.*?master(?:\s*v?\d+)?\)$").unwrap(),

        Regex::new(r"\s*[-(]?\s*mstr(?:\s*v?\d+)?\s*\)?$").unwrap(),

        Regex::new(r"\s*[-]?\s*master(?:\s*v?\d+)?$").unwrap(),

        Regex::new(r"\s+kupla\s+master(?:\s*v?\d+|\d+)?$").unwrap(),

        Regex::new(r"\s*\(.*?master(?:\s*v?\d+)?\)(?:\s*\(\d+\))+$").unwrap(),
    ];
    static ref ID_PATTERN: Regex = Regex::new(r"^[a-z]\d[ .]").unwrap();
}

impl Info {

    pub fn to_entry(&self) -> String {
        let mut entry = self.full_path.clone();

        if self.custom_name {
            entry.push('!');
            entry.push_str(&self.display_name);
        }

        if let Some(url) = &self.art_url {
            if !url.is_empty() {
                entry.push('!');
                entry.push_str(url);
            }
        }

        entry
    }

    fn decode_url(text: &str) -> String {

        #[allow(clippy::tuple_array_conversions)]
        form_urlencoded::parse(text.as_bytes())
            .map(|(key, val)| [key, val].concat())
            .collect()
    }

    fn extract_metadata(data: &Bytes) -> Metadata {
        debug_log!("tracks.rs - extract_metadata: start extracting");
        let cursor = Cursor::new(data.clone());

        let Ok(probe) = Probe::new(cursor).guess_file_type() else {
            debug_log!("tracks.rs - extract_metadata: guess_file_type failed");
            return Metadata::default();
        };

        let Ok(tagged_file) = probe.read() else {
            debug_log!("tracks.rs - extract_metadata: read tagged_file failed");
            return Metadata::default();
        };

        let Some(tag) = tagged_file.primary_tag().or_else(|| tagged_file.first_tag()) else {
            debug_log!("tracks.rs - extract_metadata: no tags found");
            return Metadata::default();
        };

        let title = tag.title().as_deref().map(ToString::to_string);
        let artist = tag.artist().as_deref().map(ToString::to_string);
        debug_log!("tracks.rs - extract_metadata: title_present={} artist_present={}", title.is_some(), artist.is_some());

        Metadata { title, artist }
    }

    fn format_name(name: &str) -> eyre::Result<String, Error> {
        let path = Path::new(name);

        let name = path
            .file_stem()
            .and_then(|x| x.to_str())
            .ok_or((name, error::Kind::InvalidName))?;

        let name = Self::decode_url(name).to_lowercase();
        let mut name = name
            .replace("masster", "master")
            .replace("(online-audio-converter.com)", "")
            .replace('_', " ");

        for regex in MASTER_PATTERNS.iter() {
            name = regex.replace(&name, "").to_string();
        }

        name = ID_PATTERN.replace(&name, "").to_string();

        let name = name
            .replace("13lufs", "")
            .to_case(Case::Title)
            .replace(" .", "")
            .replace(" Ft ", " ft. ")
            .replace("Ft.", "ft.")
            .replace("Feat.", "ft.")
            .replace(" W ", " w/ ");

        let mut skip = 0;

        for character in name.as_bytes() {
            if character.is_ascii_digit()
                || *character == b'.'
                || *character == b')'
                || *character == b'('
            {
                skip += 1;
            } else {
                break;
            }
        }

        if skip == name.len() {
            Ok(name.trim().to_string())
        } else {

            #[allow(clippy::string_slice)]
            Ok(String::from(name[skip..].trim()))
        }
    }

    pub fn new(
        name: TrackName,
        full_path: String,
        decoded: &DecodedData,
        data: Option<&Bytes>,
        art_url: Option<&str>,
    ) -> eyre::Result<Self, Error> {
        let (metadata, color_palette, raw_data) = if let Some(d) = data {

            let palette = crate::player::ui::cover::extract_color_palette(d);
            (Self::extract_metadata(d), palette, Some(Arc::new(d.clone())))
        } else {
            (Metadata::default(), None, None)
        };

        let (display_name, custom_name) = match name {
            TrackName::Formatted(custom) => (custom, true),
            TrackName::Raw(raw) => {

                if let (Some(title), Some(artist)) = (&metadata.title, &metadata.artist) {
                    (format!("{} by {}", title, artist), false)
                } else if let Some(ref title) = metadata.title {
                    (title.to_string(), false)
                } else {
                    (Self::format_name(&raw)?, false)
                }
            }
        };

        let width = display_name.graphemes(true).count();

        Ok(Self {
            duration: decoded.total_duration(),
            full_path,
            custom_name,
            display_name,
            width,
            metadata,
            color_palette,
            art_url: art_url.map(ToString::to_string),
            raw_data,
        })
    }

    pub fn new_streaming(name: TrackName, full_path: String, decoded: &DecodedData) -> eyre::Result<Self, Error> {
        Self::new(name, full_path, decoded, None, None)
    }
}

pub struct DecodedTrack {

    pub info: Info,

    pub data: DecodedData,
}

impl DecodedTrack {

    pub fn new(track: QueuedTrack) -> eyre::Result<Self, Error> {
        match track.data {
            TrackData::Full(bytes) => {
                let reader: Box<dyn ReadSeek + Send + Sync> = Box::new(Cursor::new(bytes.clone()));
                let data: DecodedData = Decoder::new(reader).track(track.full_path.clone())?;
                let info = Info::new(track.name, track.full_path, &data, Some(&bytes), track.art_url.as_deref())?;
                Ok(Self { info, data })
            }
            TrackData::Streaming(buffer) => {

                let snapshot_src = buffer.clone();
                let reader: Box<dyn ReadSeek + Send + Sync> = Box::new(GrowingReader::new(buffer));
                let data: DecodedData = Decoder::new(reader).track(track.full_path.clone())?;

                let snapshot = snapshot_src.snapshot(1024 * 1024);
                let data_opt = if snapshot.is_empty() { None } else { Some(snapshot) };
                let info = if let Some(bytes) = data_opt.as_ref() {
                    Info::new(track.name, track.full_path, &data, Some(bytes), track.art_url.as_deref())?
                } else {
                    Info::new_streaming(track.name, track.full_path, &data)?
                };
                Ok(Self { info, data })
            }
        }
    }
}
