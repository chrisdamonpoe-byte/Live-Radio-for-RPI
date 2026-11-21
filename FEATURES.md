# Live Radio

**Live radio for your Raspberry Pi: hundreds of stations, no account, one tap to play.**

## Description

Live Radio is a lightweight desktop radio player for Raspberry Pi OS. It brings
hundreds of live AM and FM stations from across the US to your Pi, across more
than 40 genres: Top 40, country, rock, hip-hop, news, sports, Spanish-language
and more. You don't need an account or a sign-in.

Browse by genre, search by name, call sign, city or frequency, and start a
station with one tap. Each station shows its own logo, and the player shows the
song or show that's on air now. Save your favourites with a heart, and your
recently played stations stay on the Home screen.

The app is built for small screens. On a normal monitor you get a sidebar
layout. Rotate the screen or use a tall touchscreen, and it switches by itself
to a phone-style layout with tabs along the bottom. It installs with
Raspberry Pi OS's own Package Installer, adds itself to the Sound & Video menu,
and uses little memory.

_Live Radio is an independent player. It is not affiliated with or endorsed by
iHeartMedia. It plays the public live streams that stations publish online._

## Features

### Listening
- **Hundreds of live stations** from across the US: about 240 of the most popular load at start-up, and each genre you open adds up to 60 more
- **One-tap play** from any station tile, with a large play/pause button in the player bar
- **Now playing:** the current song or show title, updated every 30 seconds
- **Live timer** showing how long you've been listening
- **Volume slider and mute**, remembered between sessions
- **Reliable streaming:** uses the HLS stream first, falls back to the direct stream, and buffers to ride out network hiccups
- **No account and no sign-in.** The app adds no ads of its own (stations still play their usual on-air ads).

### Finding stations
- **Home:** genre shortcuts, Recently Played and Top Stations at a glance
- **Radio:** browse all 40+ genres, with up to 60 stations per genre
- **Search:** instant results by station name, call sign, city, genre, description or frequency, across every station loaded so far
- **Station cards** show the logo, frequency and band (e.g. "100.3 FM") and the city

### Your stations
- **Favourites:** tap the heart on a station or in the player to save it
- **Recently Played:** your last 12 stations, kept on the Home screen
- Saved stations update their stream links and logos by themselves, so they keep working

### Looks and layout
- **Real station logos**, downloaded in the background and kept on disk so they appear straight away next time
- **Dark theme** that's easy on the eyes on a TV or a small display
- **Automatic portrait mode** when the window is taller than it is wide or narrower than 760 px:
  - tabs move to a bar along the bottom
  - a compact player bar
  - genre chips and Recently Played you can swipe sideways
  - station grid reflows to fit
- **Opens maximised** on a portrait monitor
- **Full-screen mode** (`live-radio --fullscreen`) with a large **Return to Player** button, for kiosk setups or launching from another app
- **Works with touch:** drag to scroll, large tap targets
- Window can be as small as 360×480

### Keyboard
- **Space:** play / pause
- **Ctrl+F:** jump to Search

### On the Pi
- **Installs with the built-in Package Installer:** double-click the `.deb`, or run `sudo apt install ./live-radio_1.1.1_arm64.deb`
- Dependencies (mpv, curl) are pulled in automatically
- **Runs on Raspberry Pi OS Bookworm and Bullseye** (64-bit, arm64), on X11 or Wayland
- Adds a **menu entry and icon** under Sound & Video
- Small and fast: a native app (about 8 MB), with no browser or Electron underneath

## Requirements
- Raspberry Pi 3, 4, 5 or 400 running 64-bit Raspberry Pi OS (Bullseye or newer)
- An internet connection
- Speakers, headphones or HDMI audio
