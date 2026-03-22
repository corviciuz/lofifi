use std::path::Path;
use std::collections::{HashMap, HashSet};
use crossterm::style::Stylize as _;

use bytes::Bytes;
use eyre::Result;
use lofty::{prelude::*, probe::Probe, tag::{Tag, TagType, TagItem, ItemValue}, picture::{Picture, PictureType, MimeType}, config::WriteOptions};
use reqwest::Client;
use tokio::{fs, io::AsyncWriteExt};
use chrono::Datelike as _;

use crate::{
    bandcamp::discography::is_album_excluded,
    bandcamp::DiscographyParser,
    debug_log,
    tracks::{
        cache::{self, BandcampCache, CachedDiscographyItem, CachedTrackInfo},
        utils::{current_timestamp, hash_string},
    },
};
use futures::future::join_all;
use std::sync::Arc;
use tokio::sync::Mutex;

pub async fn run(out: &Path, base_url: &str, _timeout: u64, _debug: bool, fetch_lyrics: bool, filter_names: Option<String>, create_playlist: bool, nodl: bool, complete: bool) -> Result<()> {
    debug_log!("check.rs - run: start base_url={} out={} ", base_url, out.display());
    if !out.exists() {
        if nodl {
            println!("Output directory {} does not exist. All albums will be reported as missing from disk.", out.display());
        } else {
            fs::create_dir_all(out).await?;
        }
    }

    let client = DiscographyParser::create_http_client()?;

    let normalized_base = if base_url.contains("/album/") {
        base_url.split("/album/").next().unwrap_or(base_url).trim_end_matches('/').trim_end_matches("/music")
    } else if base_url.contains("/track/") {
        base_url.split("/track/").next().unwrap_or(base_url).trim_end_matches('/').trim_end_matches("/music")
    } else {
        base_url.trim_end_matches('/').trim_end_matches("/music")
    };

    let data_dir = crate::cache_dir()?;
    println!("Cache dir: {}", data_dir.display());

    let url_hash = hash_string(base_url);
    let existing_cache_path = cache::find_existing_cache_path(&data_dir, url_hash);
    if let Some(path) = &existing_cache_path {
        println!("Found cache file: {}", path.display());
    } else {
        println!("No cache found for URL hash {} (base URL: {}), creating new...", url_hash, base_url);
    }

    let mut existing_cache: BandcampCache = if let Some(path) = &existing_cache_path {
        if let Some(s) = BandcampCache::read_gz_to_string(path).await {
            serde_json::from_str(&s).unwrap_or_else(|_| BandcampCache::new(normalized_base.to_string(), Vec::new()))
        } else if let Ok(cached_content) = fs::read_to_string(path).await {
            serde_json::from_str(&cached_content).unwrap_or_else(|_| BandcampCache::new(normalized_base.to_string(), Vec::new()))
        } else {
            BandcampCache::new(normalized_base.to_string(), Vec::new())
        }
    } else {
        BandcampCache::new(normalized_base.to_string(), Vec::new())
    };

    println!("Checking Bandcamp cache for updates...");
    let mut items = DiscographyParser::get_discography(&client, normalized_base).await?;
    items.retain(|item| !is_album_excluded(item));

    if let Some(names) = filter_names {
        let targets: Vec<String> = names.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        if !targets.is_empty() {
             items.retain(|item| {
                 let n = item.name.to_lowercase();
                 targets.iter().any(|t| n.contains(t) || t.contains(&n))
             });
        }
    }
    
    // Build a map of ID -> Path by scanning the directory once
    let mut id_to_path = HashMap::new();
    let mut name_to_path = HashMap::new();
    
    fn normalize_name(s: &str) -> String {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect()
    }

    if let Ok(entries) = std::fs::read_dir(out) {
        for entry in entries.flatten() {
            if let Some(name_str) = entry.file_name().to_str() {
                // Try to extract ID from end: "Name 12345"
                if let Some(last_part) = name_str.split_whitespace().last() {
                    // Strip potential brackets if any, e.g. "{123}" or "(123)"
                    let clean_id = last_part.trim_matches(|c: char| !c.is_ascii_digit());
                    if let Ok(id) = clean_id.parse::<u64>() {
                        id_to_path.insert(id, entry.path());
                    }
                }
                // Also map normalized name for fallback
                name_to_path.insert(normalize_name(name_str), entry.path());
            }
        }
    }

    let existing_ids: HashSet<Option<u64>> = existing_cache.get_item_ids().into_iter().collect();
    let new_items: Vec<_> = items
        .into_iter()
        .filter(|it| {
            if !existing_ids.contains(&it.id) {
                if nodl {
                    println!("{} Album {} ({:?}) is missing from cache.", "[NEW]".green().bold(), it.name, it.id);
                }
                return true;
            }
            
            let expected_tracks = existing_cache.items.iter()
                .find(|cached_item| cached_item.id == it.id)
                .map(|cached_item| cached_item.tracks.as_ref().map(|t| t.len()).unwrap_or(0))
                .unwrap_or(0);
                
            let album_dir = if let Some(id) = it.id {
                if let Some(path) = id_to_path.get(&id) {
                    path.clone()
                } else {
                    let norm = normalize_name(&it.name);
                    let dir_id = out.join(sanitize(&format!("{} {}", it.name, id)));
                    let dir_simple = out.join(sanitize(&it.name));
                    
                    if dir_id.exists() {
                        dir_id
                    } else if dir_simple.exists() {
                        dir_simple
                    } else if let Some(path) = name_to_path.get(&norm) {
                        path.clone()
                    } else {
                        if nodl {
                            println!("{} Album {} ({:?}) folder does not exist on disk.", "[MISS]".dark_yellow().bold(), it.name, it.id);
                        }
                        return true;
                    }
                }
            } else {
                let norm = normalize_name(&it.name);
                if let Some(path) = name_to_path.get(&norm) {
                    path.clone()
                } else {
                    let dir = out.join(sanitize(&it.name));
                    if dir.exists() {
                        dir
                    } else {
                        if nodl {
                            println!("{} Album {} (No ID) folder does not exist on disk.", "[MISS]".dark_yellow().bold(), it.name);
                        }
                        return true;
                    }
                }
            };
            
            if complete {
                let mut opus_count = 0;
                if let Ok(mut entries) = std::fs::read_dir(&album_dir) {
                    while let Some(Ok(entry)) = entries.next() {
                        if let Some(ext) = entry.path().extension() {
                            if ext == "opus" {
                                opus_count += 1;
                            }
                        }
                    }
                }
                
                if expected_tracks > 0 && opus_count < expected_tracks {
                    if nodl {
                        println!("{} Album {} ({:?}) has {}/{} tracks.", "[INCOMPLETE]".yellow().bold(), it.name, it.id, opus_count, expected_tracks);
                    } else {
                        println!("Album {} ({:?}) exists but has {}/{} tracks. Marking for re-download.", it.name, it.id, opus_count, expected_tracks);
                    }
                    return true;
                }
            }
            
            false
        })
        .collect();

    if new_items.is_empty() {
        println!("No new albums found. Cache is up to date.");

        existing_cache.timestamp = current_timestamp();
        if let Some(path) = &existing_cache_path {
            let cache_json = serde_json::to_string(&existing_cache)?;
            BandcampCache::write_gz_string(path, &cache_json).await?;
        }
        return Ok(());
    }

    if nodl {
        println!("Found {} items needing attention (see details above). Dry-run mode: no files were downloaded or modified.", new_items.len());
        return Ok(());
    }

    println!("Found {} new items. Downloading...", new_items.len());

    let mut current_cache_path = existing_cache_path.clone();
    let mut total_downloaded_tracks = 0usize;
    let playlist_mutex = Arc::new(Mutex::new(()));

    for item in new_items {

        let album_url = item.url.clone();
        let album_dir = if let Some(id) = item.id {
            if let Some(path) = id_to_path.get(&id) {
                path.clone()
            } else {
                let norm = normalize_name(&item.name);
                if let Some(path) = name_to_path.get(&norm) {
                    path.clone()
                } else {
                    out.join(sanitize(&format!("{} {}", item.name, id)))
                }
            }
        } else {
            let norm = normalize_name(&item.name);
            if let Some(path) = name_to_path.get(&norm) {
                path.clone()
            } else {
                out.join(sanitize(&item.name))
            }
        };
        fs::create_dir_all(&album_dir).await?;

        let album_artist = extract_album_artist(&client, &album_url).await.unwrap_or_else(|| "Lofi Girl".to_string());

        let album_date = extract_date_from_album(&client, &album_url).await;
        let tracks = DiscographyParser::get_album_tracks(&client, &album_url).await.unwrap_or_default();
        let track_count = tracks.len();
        if track_count == 0 { continue; }

        let original_image_url = item.image_url.clone();
        let cover_url_opt = item.image_url.as_ref().map(|u| u.replace("_9.jpg", "_10.jpg"));
        let cover_bytes: Option<Bytes> = match &cover_url_opt {
            Some(url) if url.starts_with("http") => {
                match client.get(url).send().await {
                    Ok(resp) => resp.bytes().await.ok(),
                    Err(_) => None,
                }
            }
            _ => None,
        };
        let cover_bytes = Arc::new(cover_bytes);

        let shared_album_info = Arc::new((item.name.clone(), album_artist, album_date));
        let shared_out = out.to_path_buf();

        let mut tasks: Vec<tokio::task::JoinHandle<eyre::Result<(usize, CachedTrackInfo)>>> = Vec::new();

        println!("Processing album: {} ({} tracks)", item.name, track_count);

        for (idx, t) in tracks.iter().enumerate() {
             let client = client.clone();
             let track = t.clone();
             let album_dir = album_dir.clone();
             let cover_bytes = cover_bytes.clone();
             let shared_album_info = shared_album_info.clone();
             let shared_out = shared_out.clone();
             let playlist_mutex = playlist_mutex.clone();

             tasks.push(tokio::spawn(async move {
                let (raw_album_name, album_artist, album_date) = &*shared_album_info;
                let (mut artist, mut title) = (
                    track.artist.clone().unwrap_or_else(|| album_artist.clone()),
                    track.name.clone(),
                );

                if let Ok(re) = regex::Regex::new(r"(?i)\s*\((?:ft\.|feat\.|featuring)\s+([^)]+)\)") {
                    if let Some(cap) = re.captures(&title.clone()) {
                        if let Some(feat) = cap.get(1) {
                            artist = format!("{} ft. {}", artist, feat.as_str());
                            title = re.replace(&title, "").trim().to_string();
                        }
                    }
                }

                let nn = if track_count > 9 { format!("{:02}", idx + 1) } else { format!("{}", idx + 1) };

                let sanitized_artist = sanitize(&artist);
                let sanitized_title = sanitize(&title);
                let filename_opus = sanitize(&format!("{}. {} - {}.opus", nn, &sanitized_artist, &sanitized_title));
                let path_opus = album_dir.join(&filename_opus);
                let tmp_src = album_dir.join(format!("{}.tmp_download", nn));

                let mut play_url = track.url.clone();
                if (play_url.contains("bandcamp.com") || play_url.contains("bcbits.com")) && play_url.contains("/track/") {
                    if let Ok(Some(s)) = DiscographyParser::get_track_stream_url(&client, &play_url).await { play_url = s; }
                }

                println!("  Starting: {} - {}", idx + 1, title);
                download_to_file_with_progress(&client, &play_url, &tmp_src).await?;

                let lyrics_text = if fetch_lyrics && track.url.contains("/track/") {
                    extract_lyrics(&client, &track.url).await
                } else {
                    None
                };

                let tmp_src_blocking = tmp_src.clone();
                let path_opus_blocking = path_opus.clone();
                let title_clone = title.clone();
                let artist_clone = artist.clone();
                let raw_album_name_clone = raw_album_name.clone();
                let album_artist_clone = album_artist.clone();
                let date_clone = *album_date;
                let lyrics_clone = lyrics_text.clone();
                let cover_bytes_clone = cover_bytes.clone();

                tokio::task::spawn_blocking(move || -> Result<()> {
                     let src_kbps = detect_source_bitrate_kbps(&tmp_src_blocking);
                     let target_kbps = src_kbps.map(compute_opus_target_bitrate_kbps).unwrap_or(96);

                     let mut cmd = std::process::Command::new("ffmpeg");
                     cmd.arg("-y").arg("-hide_banner").arg("-nostats").arg("-loglevel").arg("error")
                        .arg("-i").arg(&tmp_src_blocking)
                        .arg("-c:a").arg("libopus")
                        .arg("-vbr").arg("on")
                        .arg("-b:a").arg(format!("{}k", target_kbps))
                        .arg("-map_metadata").arg("0")
                        .arg("-map_chapters").arg("-1")
                        .arg(&path_opus_blocking);

                     let status = cmd.status()?;
                     if status.success() {
                         let _ = std::fs::remove_file(&tmp_src_blocking);
                     } else {
                         return Err(eyre::eyre!("ffmpeg failed"));
                     }

                     write_tags_only(&path_opus_blocking, &*cover_bytes_clone, &title_clone, &artist_clone, &raw_album_name_clone, &album_artist_clone, (idx + 1) as u32, date_clone, lyrics_clone.as_deref())?;
                     Ok(())
                }).await??;

                if create_playlist {
                    let _lock = playlist_mutex.lock().await;
                    if let Err(e) = add_track_to_playlist(&shared_out, &path_opus, &artist, &title).await {
                        eprintln!("  Warning: Failed to add track to playlist: {}", e);
                    }
                }

                println!("  Done: {} - {}", idx + 1, title);

                Ok((idx, CachedTrackInfo { name: title, url: track.url, artist: Some(artist) }))
             }));
        }

        let results = join_all(tasks).await;
        let mut cached_tracks = Vec::new();
        let mut success_count = 0;

        for res in results {
             match res {
                 Ok(Ok((idx, info))) => {
                     cached_tracks.push((idx, info));
                     success_count += 1;
                 },
                 Ok(Err(e)) => eprintln!("  Track task failed: {}", e),
                 Err(e) => eprintln!("  Join error: {}", e),
             }
        }
        cached_tracks.sort_by_key(|(i, _)| *i);
        let final_cached: Vec<CachedTrackInfo> = cached_tracks.into_iter().map(|(_, info)| info).collect();
        total_downloaded_tracks += success_count;

        let new_cached_item = CachedDiscographyItem {
            id: item.id,
            item_type: item.item_type,
            name: item.name,
            url: album_url,
            image_url: original_image_url,
            tracks: Some(final_cached),
        };

        existing_cache.add_items(vec![new_cached_item]);

        let cache_json = serde_json::to_string(&existing_cache)?;
        let new_cache_key = format!("bandcamp_cache_{}_{}", url_hash, existing_cache.items_hash);
        let new_cache_path_gz = data_dir.join(format!("{}.cache.gz", new_cache_key));
        BandcampCache::write_gz_string(&new_cache_path_gz, &cache_json).await?;

        if let Some(old_path) = &current_cache_path {
            if &new_cache_path_gz != old_path {
                let _ = fs::remove_file(old_path).await;
            }
        }
        current_cache_path = Some(new_cache_path_gz);
    }

    println!("Downloaded {} new tracks", total_downloaded_tracks);
    Ok(())
}

fn sanitize(name: &str) -> String {

    let mut s = name.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "");

    s = s.chars().map(|c| if c.is_whitespace() { ' ' } else { c }).collect();

    while s.contains("  ") {
        s = s.replace("  ", " ");
    }

    s = s.trim().to_string();
    s.truncate(200);
    s
}

fn detect_source_bitrate_kbps(path: &Path) -> Option<u32> {
    let out = std::process::Command::new("ffprobe")
        .arg("-v").arg("error")
        .arg("-select_streams").arg("a:0")
        .arg("-show_entries").arg("stream=bit_rate")
        .arg("-of").arg("default=nw=1:nk=1")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { return None; }
    if let Ok(bps) = s.parse::<u64>() {
        if bps > 0 { return Some((bps as u32) / 1000); }
    }
    None
}

fn compute_opus_target_bitrate_kbps(src_kbps: u32) -> u32 {

    let mut target = ((src_kbps as f32) * 0.8).round() as u32;
    if target < 64 { target = 64; }
    if target > 128 { target = 128; }

    let steps: [u32; 6] = [64, 80, 96, 112, 128, 160];
    let mut best = steps[0];
    let mut best_diff = best.abs_diff(target);
    for s in steps.iter().copied() {
        let d = s.abs_diff(target);
        if d < best_diff { best = s; best_diff = d; }
    }

    if best > 128 { 128 } else { best }
}

async fn extract_album_artist(client: &Client, album_url: &str) -> Option<String> {

    if let Ok(resp) = client.get(album_url).send().await {
        if let Ok(html) = resp.text().await {

            if let Some(start) = html.find("var TralbumData = ") {
                if let Some(end) = html[start..].find("};") {
                    let json_str = &html[start + 18..start + end + 1];
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {
                        if let Some(artist) = data.get("artist").and_then(|v| v.as_str()) {
                            let name = artist.trim().to_string();
                            if !name.is_empty() { return Some(name); }
                        }
                    }
                }
            }

            for pattern in &[r#"data-tralbum=\"([^\"]+)\""#, r#"data-tralbum='([^']+)'"#] {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if let Some(cap) = re.captures(&html) {
                        if let Some(json_str) = cap.get(1) {
                            let decoded = html_escape::decode_html_entities(json_str.as_str());
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&decoded) {
                                if let Some(artist) = parsed.get("artist").and_then(|v| v.as_str()) {
                                    let name = artist.trim().to_string();
                                    if !name.is_empty() { return Some(name); }
                                }
                            }
                        }
                    }
                }
            }

            for pattern in &[r#"data-band=\"([^\"]+)\""#, r#"data-band='([^']+)'"#] {
                if let Ok(re) = regex::Regex::new(pattern) {
                    if let Some(cap) = re.captures(&html) {
                        if let Some(json_str) = cap.get(1) {
                            let decoded = html_escape::decode_html_entities(json_str.as_str());
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&decoded) {
                                if let Some(name) = parsed.get("name").and_then(|v| v.as_str()) {
                                    let name = name.trim().to_string();
                                    if !name.is_empty() { return Some(name); }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Ok(tracks) = DiscographyParser::get_album_tracks(client, album_url).await {
        if let Some(a) = tracks.iter().filter_map(|t| t.artist.clone()).find(|a| !a.is_empty()) {
            return Some(a);
        }
    }
    None
}

async fn extract_date_from_album(client: &Client, album_url: &str) -> Option<chrono::NaiveDate> {
    if let Ok(resp) = client.get(album_url).send().await {
        if let Ok(html) = resp.text().await {

            let re = regex::Regex::new(r"(?i)released\s+([a-zA-Z]+\s+\d{1,2},\s+\d{4})").ok()?;
            if let Some(cap) = re.captures(&html) {
                if let Some(date_str) = cap.get(1) {
                    let date_str = date_str.as_str().trim();

                    if let Ok(dt) = chrono::NaiveDate::parse_from_str(date_str, "%B %d, %Y") {
                        return Some(dt);
                    }
                }
            }

             if let Some(start) = html.find("var TralbumData = ") {
                if let Some(end) = html[start..].find("};") {
                    let json_str = &html[start + 18..start + end + 1];
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(json_str) {

                         if let Some(rd) = data.get("release_date").and_then(|v| v.as_str()) {
                             if let Ok(dt) = chrono::NaiveDate::parse_from_str(&rd[..11], "%d %b %Y") {
                                 return Some(dt);
                             }
                         }
                    }
                }
            }
        }
    }
    None
}

async fn download_to_file_with_progress(client: &Client, url: &str, path: &Path) -> Result<()> {
    use futures::StreamExt as _;

    if let Some(parent) = path.parent() { tokio::fs::create_dir_all(parent).await?; }

    let mut req = client.get(url);
    if url.contains("bandcamp.com") || url.contains("bcbits.com") {
        req = req.header(reqwest::header::REFERER, "https://bandcamp.com/");
    }

    let resp = req.send().await?;
    let mut stream = resp.bytes_stream();

    let mut file = fs::File::create(path).await?;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
    }

    Ok(())
}

fn get_audio_duration_seconds(path: &Path) -> Option<u32> {
    let out = std::process::Command::new("ffprobe")
        .arg("-v").arg("error")
        .arg("-show_entries").arg("format=duration")
        .arg("-of").arg("default=noprint_wrappers=1:nokey=1")
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() { return None; }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { return None; }
    if let Ok(duration_f64) = s.parse::<f64>() {
        if duration_f64 > 0.0 {
            return Some(duration_f64.round() as u32);
        }
    }
    None
}

async fn add_track_to_playlist(
    out_dir: &Path,
    track_path: &Path,
    artist: &str,
    title: &str,
) -> Result<()> {
    let playlist_path = out_dir.join("#LoFiFi.m3u8");
    let duration = get_audio_duration_seconds(track_path).unwrap_or(0);

    let absolute_track_path = fs::canonicalize(track_path).await.unwrap_or_else(|_| track_path.to_path_buf());
    let mut track_path_str = absolute_track_path.display().to_string();

    if track_path_str.starts_with("\\\\?\\") {
        track_path_str = track_path_str[4..].to_string();
    }

    let entry = format!("#EXTINF:{},{}\n{}\n", duration, format!("{} - {}", artist, title), track_path_str);

    let existing_content = if playlist_path.exists() {
        fs::read_to_string(&playlist_path).await.unwrap_or_default()
    } else {
        String::new()
    };

    let track_filename = track_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let lines: Vec<&str> = existing_content.lines().collect();
    let mut track_exists = false;
    for line in &lines {

        if !line.trim_start().starts_with('#') && !line.trim().is_empty() {
            let mut existing_path = line.trim().to_string();

            if existing_path.starts_with("\\\\?\\") {
                existing_path = existing_path[4..].to_string();
            }

            if existing_path.ends_with(track_filename) {
                track_exists = true;
                break;
            }

            let normalized_existing = existing_path.replace('/', "\\");
            let normalized_new = track_path_str.replace('/', "\\");
            if normalized_existing == normalized_new || existing_path == track_path_str {
                track_exists = true;
                break;
            }
        }
    }

    if track_exists {

        println!("  Track already in playlist, skipping: {}", track_filename);
        return Ok(());
    }

    let mut cleaned = existing_content.trim_start();

    if cleaned.starts_with('\u{FEFF}') {
        cleaned = &cleaned[3..];
    }

    let cleaned_lines: Vec<&str> = cleaned
        .lines()
        .filter(|line| {
            let trimmed = line.trim();

            !trimmed.eq_ignore_ascii_case("#EXTM3U")
        })
        .collect();

    let cleaned_content = cleaned_lines.join("\n").trim().to_string();

    let mut content = if cleaned_content.is_empty() {
        "#EXTM3U\n".to_string()
    } else {
        format!("#EXTM3U\n{}\n", cleaned_content)
    };

    if !content.ends_with('\n') && content.len() > 7 {
        content.push('\n');
    }

    let insert_pos = if content.starts_with("#EXTM3U") {
        let after_header = 7;
        let after_newlines = content[after_header..]
            .chars()
            .take_while(|c| *c == '\n' || *c == '\r')
            .count();
        after_header + after_newlines
    } else {
        0
    };

    content.insert_str(insert_pos, &entry);

    fs::write(&playlist_path, &content).await?;
    println!("  Added to playlist: {} - {} (playlist: {})", artist, title, playlist_path.display());

    Ok(())
}

fn write_tags_only(
    path: &Path,
    cover: &Option<Bytes>,
    title: &str,
    artist: &str,
    album: &str,
    album_artist: &str,
    track_number: u32,
    date: Option<chrono::NaiveDate>,
    lyrics: Option<&str>,
) -> Result<()> {

    if let Ok(mut tagged) = Probe::open(path).and_then(|p| p.read()) {
        let tag_type = match path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase()) {
            Some(ref ext) if ext == "opus" || ext == "ogg" => TagType::VorbisComments,
            _ => TagType::Id3v2,
        };
        let mut tag = tagged
            .primary_tag()
            .map(|t| t.to_owned())
            .unwrap_or_else(|| Tag::new(tag_type));

        tag.insert(TagItem::new(ItemKey::TrackTitle, ItemValue::Text(title.to_string())));
        tag.insert(TagItem::new(ItemKey::TrackArtist, ItemValue::Text(artist.to_string())));
        tag.insert(TagItem::new(ItemKey::AlbumTitle, ItemValue::Text(album.to_string())));
        tag.insert(TagItem::new(ItemKey::AlbumArtist, ItemValue::Text(album_artist.to_string())));
        tag.insert(TagItem::new(ItemKey::TrackNumber, ItemValue::Text(track_number.to_string())));
        if let Some(d) = date {
            let ys = d.year().to_string();
            let date_str = d.format("%Y-%m-%d").to_string();
            tag.insert(TagItem::new(ItemKey::Year, ItemValue::Text(ys)));
            tag.insert(TagItem::new(ItemKey::RecordingDate, ItemValue::Text(date_str.clone())));
            tag.insert(TagItem::new(ItemKey::OriginalReleaseDate, ItemValue::Text(date_str)));
        }
        tag.insert(TagItem::new(ItemKey::Genre, ItemValue::Text("Lo-Fi".to_string())));
        tag.insert(TagItem::new(ItemKey::Composer, ItemValue::Text(artist.to_string())));

        if let Some(l) = lyrics {
           tag.insert(TagItem::new(ItemKey::Lyrics, ItemValue::Text(l.to_string())));
        }

        if let Some(img) = cover {
            let picture = Picture::unchecked(img.clone().to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Jpeg)
                .build();
            tag.push_picture(picture);
        }

        tagged.insert_tag(tag);
        tagged.save_to_path(path, WriteOptions::default())?;
    }
    Ok(())
}

pub fn default_base_url() -> String {
    #[cfg(has_embedded_data)]
    {
        #[derive(serde::Deserialize)]
        struct PresavedBase {
            base_url: String,
        }
        let bytes = include_bytes!(env!("BANDCAMP_EMBEDDED_PATH"));
        let mut decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(bytes));
        let mut out = String::new();
        if std::io::Read::read_to_string(&mut decoder, &mut out).is_ok() {
            if let Ok(p) = serde_json::from_str::<PresavedBase>(&out) {
                return p.base_url;
            }
        }
    }
    "https://lofigirl.bandcamp.com".to_string()
}

async fn extract_lyrics(client: &Client, track_url: &str) -> Option<String> {
    if let Ok(resp) = client.get(track_url).send().await {
        if let Ok(html) = resp.text().await {
            let start_marker = "<div class=\"tralbumData lyricsText\">";
            if let Some(start) = html.find(start_marker) {
                let rest = &html[start + start_marker.len()..];
                if let Some(end) = rest.find("</div>") {
                    let content = &rest[..end];
                    let content = content.replace("<br>", "\n")
                                         .replace("<br/>", "\n")
                                         .replace("<br />", "\n");
                    let decoded = html_escape::decode_html_entities(&content).trim().to_string();
                    if !decoded.is_empty() {
                        return Some(decoded);
                    }
                }
            }
        }
    }
    None
}
