# download-m3u8

A Rust CLI tool that downloads HLS (m3u8) streams and converts them to mp4 files.

## Features

- Automatically selects the highest quality variant from master playlists
- Downloads segments in parallel (configurable concurrency)
- Merges segments into mp4 using ffmpeg with stream copy (no re-encoding)
- Supports both master and media playlists
- Handles relative and absolute segment URLs

## Prerequisites

- [Rust](https://rustup.rs/) (for building)
- [ffmpeg](https://ffmpeg.org/) (must be on PATH)

## Installation

```sh
cargo install --path .
```

## Usage

```sh
download-m3u8 <M3U8_URL> <OUTPUT_FILE> [-c <CONCURRENCY>]
```

### Arguments

| Argument | Description |
|---|---|
| `<M3U8_URL>` | URL of the m3u8 playlist |
| `<OUTPUT_FILE>` | Output mp4 file path |
| `-c, --concurrency` | Number of parallel downloads (default: 5) |

### Examples

```sh
# Download with default concurrency (5)
download-m3u8 https://example.com/stream/master.m3u8 video.mp4

# Download with 10 parallel connections
download-m3u8 https://example.com/stream/master.m3u8 video.mp4 -c 10
```

## How it works

1. Fetches the m3u8 playlist
2. If it's a master playlist, selects the variant with the highest bandwidth
3. Parses segment URLs from the media playlist
4. Downloads all `.ts` segments in parallel to a temp directory
5. Runs `ffmpeg -c copy` to mux segments into an mp4 container without re-encoding
