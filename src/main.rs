use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Parser;
use futures::stream::{self, StreamExt};
use url::Url;

#[derive(Parser)]
#[command(name = "download-m3u8", about = "Download m3u8 streams as mp4 files")]
struct Args {
    /// URL of the m3u8 playlist
    url: String,

    /// Output mp4 file path
    output: String,

    /// Number of parallel segment downloads
    #[arg(short, long, default_value_t = 5)]
    concurrency: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let base_url = Url::parse(&args.url).context("invalid m3u8 URL")?;
    let client = reqwest::Client::new();

    // Fetch the master playlist
    let playlist_text = client
        .get(base_url.as_str())
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    // Resolve to the media playlist (pick highest bandwidth variant if master)
    let media_url = resolve_media_playlist(&base_url, &playlist_text)?;

    let media_text = if media_url.as_str() == base_url.as_str() {
        playlist_text
    } else {
        eprintln!("Selected variant: {}", media_url);
        client
            .get(media_url.as_str())
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?
    };

    // Parse segment URLs from the media playlist
    let segment_urls = parse_segments(&media_url, &media_text)?;
    let total = segment_urls.len();
    eprintln!("Downloading {} segments (concurrency={})", total, args.concurrency);

    // Create a temp directory for segments
    let tmp_dir = tempfile::tempdir().context("failed to create temp dir")?;

    // Download segments in parallel, preserving order via indexed tuples
    let results: Vec<(usize, Result<PathBuf>)> = stream::iter(segment_urls.into_iter().enumerate())
        .map(|(i, url)| {
            let client = client.clone();
            let dir = tmp_dir.path().to_owned();
            async move {
                let seg_path = dir.join(format!("seg_{:06}.ts", i));
                let result = download_segment(&client, &url, &seg_path).await.map(|()| seg_path);
                eprintln!("  [{}/{}] {}", i + 1, total, url);
                (i, result)
            }
        })
        .buffer_unordered(args.concurrency)
        .collect()
        .await;

    // Sort by index and check for errors
    let mut indexed: Vec<(usize, PathBuf)> = Vec::with_capacity(total);
    for (i, r) in results {
        indexed.push((i, r.with_context(|| format!("segment {} failed", i))?));
    }
    indexed.sort_by_key(|(i, _)| *i);

    // Write a concat file for ffmpeg
    let concat_file = tmp_dir.path().join("concat.txt");
    let concat_content: String = indexed
        .iter()
        .map(|(_, p)| format!("file '{}'\n", p.display()))
        .collect();
    std::fs::write(&concat_file, &concat_content)?;

    // Merge with ffmpeg using stream copy (no re-encoding)
    eprintln!("Merging segments with ffmpeg...");
    run_ffmpeg(&concat_file, Path::new(&args.output))?;

    eprintln!("Done: {}", args.output);
    Ok(())
}

/// If the playlist is a master playlist, pick the variant with the highest bandwidth.
/// Otherwise return the original URL (it's already a media playlist).
fn resolve_media_playlist(base_url: &Url, text: &str) -> Result<Url> {
    let mut best_bandwidth: u64 = 0;
    let mut best_uri: Option<String> = None;
    let mut next_is_variant = false;
    let mut current_bw: u64 = 0;

    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("#EXT-X-STREAM-INF:") {
            if let Some(bw) = parse_attribute(line, "BANDWIDTH") {
                current_bw = bw.parse::<u64>().unwrap_or(0);
            }
            next_is_variant = true;
        } else if next_is_variant && !line.is_empty() && !line.starts_with('#') {
            if current_bw > best_bandwidth || best_uri.is_none() {
                best_bandwidth = current_bw;
                best_uri = Some(line.to_string());
            }
            next_is_variant = false;
        }
    }

    match best_uri {
        Some(uri) => resolve_url(base_url, &uri),
        None => Ok(base_url.clone()),
    }
}

/// Parse segment URLs from a media playlist.
fn parse_segments(base_url: &Url, text: &str) -> Result<Vec<String>> {
    let mut segments = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let url = resolve_url(base_url, line)?;
        segments.push(url.to_string());
    }

    if segments.is_empty() {
        bail!("no segments found in media playlist");
    }

    Ok(segments)
}

/// Resolve a possibly-relative URI against a base URL.
fn resolve_url(base: &Url, uri: &str) -> Result<Url> {
    if uri.starts_with("http://") || uri.starts_with("https://") {
        Ok(Url::parse(uri)?)
    } else {
        Ok(base.join(uri)?)
    }
}

/// Extract an HLS attribute value (e.g. BANDWIDTH=1234).
fn parse_attribute<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{}=", key);
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];
    let end = rest.find(',').unwrap_or(rest.len());
    let val = rest[..end].trim().trim_matches('"');
    Some(val)
}

/// Download a single segment to a file.
async fn download_segment(client: &reqwest::Client, url: &str, path: &Path) -> Result<()> {
    let bytes = client
        .get(url)
        .send()
        .await?
        .error_for_status()
        .with_context(|| format!("HTTP error for {}", url))?
        .bytes()
        .await?;

    tokio::fs::write(path, &bytes).await?;
    Ok(())
}

/// Run ffmpeg to concatenate TS segments into an mp4 with stream copy.
fn run_ffmpeg(concat_file: &Path, output: &Path) -> Result<()> {
    let status = Command::new("ffmpeg")
        .args(["-y", "-f", "concat", "-safe", "0", "-i"])
        .arg(concat_file)
        .args(["-c", "copy"])
        .arg(output)
        .status()
        .context("failed to run ffmpeg — is it installed?")?;

    if !status.success() {
        bail!("ffmpeg exited with {}", status);
    }
    Ok(())
}
