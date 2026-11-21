# How the port works

This document records the mechanics of the original Android app that the
desktop port keeps, the values that were measured from the APK, and the
places where the port makes a deliberate, documented choice. Decompiled
code is not reproduced here; class and method names are cited so the
behaviour can be checked against the target.

Target: `com.clearchannel.iheartradio.controller`, version 10.67.1
(Java/Kotlin, DEX bytecode, no native entry point).
Port: Rust + eframe/egui, one executable, `arm64` Debian package.

## 1. Account-free station catalogue

The app's own catalogue is fetched over HTTPS from the public host
`us.api.iheart.com`. The host and the `/api/v2/content` prefix occur in
the DEX string table; the exact calls were confirmed against the live
service.

| Purpose | Request | Notes |
| --- | --- | --- |
| Genres | `GET /api/v2/content/genre` | 45 genres, each `{id,name,logo,sort,display}` |
| All stations | `GET /api/v2/content/liveStations?limit=240` | total catalogue is 10,532 |
| Genre stations | `GET /api/v2/content/liveStations?genreId={id}&limit=60` | |

Measured paging behaviour: `offset` is honoured (`limit` accepts at
least 500); `page`, `start`, `skip`, `from` and `index` are ignored.
`name`, `keyword` and `title` query parameters are also ignored, while
`callLetters` matches exactly. This is why search in the port is local
over the stations already loaded instead of relying on the service.

Only these fields are used: `id`, `name`, `callLetters`, `description`,
`freq`, `streams`, `markets`, `genres`, `cume`. Fields that would need an
account (favourites, personalised recommendations, podcasts) are not
requested.

## 2. Stream selection

`LiveStationReader.getLiveStationFromV2Response` resolves the stream
keys declared by `LiveStationReader$LiveStationStreamType`. The enum's
static initialiser lists, in order:

```
pivot_hls_stream, secure_hls_stream, hls_stream,
secure_shoutcast_stream, shoutcast_stream,
secure_pls_stream, pls_stream
```

The reader tries HLS first, then SHOUTCAST, then PLS, then PIVOT. The
port follows the same preference but only uses the two families that
are publicly playable without a token:

1. `secure_hls_stream`, else `hls_stream`
2. `secure_shoutcast_stream`, else `shoutcast_stream`

Measured stream properties (station 577, `https://stream.revma.ihrhls.com/zc577`):

* master HLS playlist offers one variant at `BANDWIDTH=24000`,
  `CODECS="mp4a.40.2"`, and a media playlist with 10-second AAC
  segments;
* the progressive stream redirects to `cloud.revma.ihrhls.com`, serves
  AAC ADTS at 48 kbps / 48 kHz stereo, with `icy-metaint: 16000`;
* both are plain HTTPS and need no cookies or account.

Stations with neither family are filtered out of the catalogue. The
service is asked for 240 stations on first run (206 usable in the test
run); 60 per genre is loaded on demand.

## 3. Playback

Audio is delegated to an `mpv` child process (`--no-video`,
`--cache-secs=8`). This is a port decision: it keeps ALSA/PipeWire
headers and FFmpeg out of the build, and mpv already handles HLS and
the progressive AAC stream. The package declares `mpv` and `curl`.

* Switching stations stops the old helper and starts a new one at the
  live edge. Live radio has no seek bar in the original.
* Pause, resume and volume use mpv's JSON IPC socket on Linux
  (`--input-ipc-server`). If IPC is unavailable (Windows development
  build), pause stops the helper and play restarts it.
* Volume is persisted as a fraction; the helper receives 0-100.
* A helper that exits unexpectedly is reported in the player bar and
  the play button reconnects.

## 4. Now playing

iHeart's media playlist embeds the current item as
`#EXTINF:10,title="..." artist="..."`; the port reads the first titled
entry of the variant playlist. If that is unavailable, it falls back to
the progressive stream's ICY metadata: request `Icy-MetaData: 1`, skip
`icy-metaint` audio bytes, read the length-prefixed text block and take
`StreamTitle`.

Measured `StreamTitle` payloads have the form

```
- text="WZTU - DJCRISS - MIX 2180" song_spot="T" spotInstanceId="-1" ...
```

so the readable value is extracted from `text="..."`. Without this
clean-up the player bar would show the whole key/value payload.

The private metadata endpoint
`/api/v3/live-meta/stream/{id}/currentTrackMeta` answered 410/400 during
testing and is not used. Instead, the now-playing probe is refreshed
every 30 seconds while audio is running. The 30-second value is a port
choice; the only metadata refresh constant recovered from the APK is
`0x493e0` = 300000 ms in
`LiveStationRecentlyPlayedViewModel.refreshIfNeeded`, which governs the
recently-played list rather than the player bar. The port keeps the
300000 ms figure as the recently-played refresh reference.

## 5. Interface

Values read from the APK resources and string table:

| Item | Value |
| --- | --- |
| App background | `#262626` |
| Card surface | `#2E2E2E` |
| Primary red (`ihr_red_600`) | `#C6002B` |
| Secondary red (`ihr_red_550`) | `#CC032E` |
| Navigation labels | Home, Radio, Library, Search, Genres, Favorites, Live Station |

The port keeps the dark surface, the red accent and the rail-plus-grid
layout, and uses Home / Radio / Favorites / Search in the rail. The
original "Library" is replaced by Favorites because saved podcasts and
account content are out of scope. All measurements of the window
(1180x780 default, 92 px rail, 90 px player bar, six-column grid,
102 px recent tiles) are port estimates, chosen to fit a 720p/1080p Pi
desktop; fonts use egui's built-in proportional face.

Artwork is procedural: a seeded two-colour gradient, a soft glow, rings
and the station's call sign. No logo, photo or brand asset from the APK
is shipped. The wordmark is the neutral "LIVE RADIO".

Behaviour:

* Home shows recently played, the first 30 stations and genre chips.
* Radio has a genre list on the left and a scrollable grid on the right.
* Favorites shows saved stations; the heart on a tile or in the player
  bar toggles membership.
* Search filters the loaded catalogue by name, call sign, city, genre,
  description and frequency (Ctrl+F focuses the field).
* Space toggles playback when no text field has focus.
* A click on a tile starts that station; clicking the current station's
  tile pauses/resumes it.

## 6. Saved state

`Store` writes one JSON document with favourites, the 12 most recent
stations and the volume. It lives at
`$XDG_CONFIG_HOME/live-radio/state.json`, or
`~/.config/live-radio/state.json` when `XDG_CONFIG_HOME` is unset.
Windows development builds use `%APPDATA%\LiveRadio\state.json`.
Fresh stream URLs are merged into saved records whenever the catalogue
loads, so saved stations keep working after a broadcaster changes a
redirect. Nothing is uploaded and no account is involved.

## 7. Packaging

The deliverable is a Debian package for `arm64` Raspberry Pi OS:

* binary dynamically linked against glibc 2.28 (verified NEEDED:
  `libc.so.6`, `libm.so.6`, `libpthread.so.0`, `libdl.so.2`);
* GUI libraries are loaded at runtime by winit/glutin, hence the
  `libx11-6`, `libxcursor1`, `libxrandr2`, `libxi6`, `libxkbcommon0`,
  `libwayland-*`, `libegl1`, `libgl1`, `libgles2` dependencies;
* `mpv` supplies audio and HLS, `curl` supplies HTTPS;
* desktop entry and hicolor icons are installed, so the built-in
  Package Installer and the application menu both find the app.

A static musl build was tested for compilation but rejected: a static
musl process cannot `dlopen` the system's glibc-linked GL/X11/Wayland
libraries that eframe needs. The gnu build with a zig-provided glibc
2.28 sysroot is used instead.

Sound and now-playing depend on the broadcaster remaining reachable
and on the station's stream being available in the listener's region.
No ad skipping, account features, podcasts, or recording are included.
