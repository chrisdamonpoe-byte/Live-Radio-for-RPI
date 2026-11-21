# Live Radio

A small desktop radio player for Raspberry Pi OS. It lists the live
stations published by iHeart's public API, plays their public HLS or AAC
streams, and needs no broadcaster account.

The original Android app was studied to recover the catalogue calls,
stream preference and interface values; see `MECHANICS.md` and
`mechanics.json`. No logos, photographs or long text ship in the package:
station logos are fetched from the public image service while the app runs
and cached in `~/.cache/live-radio/logos`; a generated tile shows until one
loads.

The layout switches to portrait (tabs along the bottom, compact player) when
the window is taller than it is wide or narrower than 760 px, so it suits a
rotated touchscreen. On a portrait monitor the window opens maximised.

## Install (Raspberry Pi 4, 64-bit Raspberry Pi OS)

The built package is `dist/live-radio_1.1.1_arm64.deb`.

1. Copy it to the Pi.
2. Open it in Files and choose **Install** (the built-in Package
   Installer), or run:

   ```sh
   sudo apt install ./live-radio_1.1.1_arm64.deb
   ```

The installer pulls in `mpv` (playback), `curl` (HTTPS) and the usual
X11/Wayland/OpenGL runtime libraries. An internet connection is needed
both for the install and for listening.

Remove it with:

```sh
sudo apt remove live-radio
```

Saved favourites live in `~/.config/live-radio/state.json` and are left
in place when the package is removed.

A 32-bit Raspberry Pi OS installation cannot run this arm64 package;
use a 64-bit image.

## Using it

* **Home** — recently played, genre shortcuts and the most popular
  stations.
* **Radio** — browse every loaded station or pick a genre.
* **Favorites** — everything you hearted. The heart on a tile or in the
  player bar toggles it.
* **Search** — filters the loaded catalogue by name, call sign, city,
  genre or frequency. `Ctrl+F` jumps to the field.
* `Space` toggles playback while no text field has focus.

The bottom bar shows the current station, song title when the stream
publishes it, live time, play/pause and volume. Radio is live, so there
is deliberately no seek bar.

## Build from source

On the Pi itself (native build):

```sh
cargo build --release
```

The binary is `target/release/live-radio`. The package build in
`packaging/` only needs the release binary, GNU `ar`, `tar` and `xz`.

Cross-compiling from Windows uses `cargo-zigbuild` against glibc 2.28:

```sh
cargo zigbuild --release --target aarch64-unknown-linux-gnu.2.28
powershell -ExecutionPolicy Bypass -File packaging/build-deb.ps1
```

## Verification

`--screenshot <file.png>` renders about 120 frames, writes a PNG and
exits. It opens without focus (and behind other windows on Windows) so
it can be used as a visual smoke test without touching audio.

Known limits: station availability varies by region, only live stations
are included (no podcasts or account features), and no physical Pi was
available during development, so the package was validated by
inspection and by a native Windows build of the same UI.
