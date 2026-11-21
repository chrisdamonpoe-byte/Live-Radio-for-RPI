//! Audio playback through `mpv`.
//!
//! mpv is used as a helper process: it understands HLS and the progressive
//! AAC streams, and it keeps the app free of audio-stack build dependencies.
//! On Linux a small JSON IPC socket lets us pause and change volume without
//! restarting the stream.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::api::Station;

pub struct Player {
    pub station: Option<Station>,
    pub volume: f32,
    pub paused: bool,
    pub status: String,
    pub failed: bool,
    pub now_playing: Option<String>,
    last_volume: f32,
    preview: bool,
    child: Option<Child>,
    started: Instant,
    #[cfg(unix)]
    socket: Option<std::path::PathBuf>,
}

impl Player {
    pub fn new(volume: f32) -> Self {
        Self {
            station: None,
            volume: volume.clamp(0.0, 1.0),
            paused: false,
            status: "Choose a station to start listening".to_string(),
            failed: false,
            now_playing: None,
            last_volume: volume.clamp(0.0, 1.0),
            preview: false,
            child: None,
            started: Instant::now(),
            #[cfg(unix)]
            socket: None,
        }
    }

    pub fn active(&self) -> bool {
        self.child.is_some()
    }

    pub fn playing(&self) -> bool {
        self.preview || (self.child.is_some() && !self.paused)
    }

    /// Show a tuned-in station without starting audio (used by screenshots).
    pub fn preview(&mut self, station: &Station) {
        self.preview = true;
        self.station = Some(station.clone());
        self.paused = false;
        self.failed = false;
        self.status = "Live".to_string();
        self.now_playing = None;
        self.started = Instant::now();
    }

    pub fn station_id(&self) -> Option<u64> {
        self.station.as_ref().map(|s| s.id)
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn play(&mut self, station: &Station) {
        self.stop_process();
        self.station = Some(station.clone());
        self.paused = false;
        self.now_playing = None;
        self.failed = false;
        self.preview = false;

        let url = station.stream().to_string();
        if url.is_empty() {
            self.status = "This station has no playable stream".to_string();
            self.failed = true;
            return;
        }

        let volume = (self.volume * 100.0).round().clamp(0.0, 100.0) as i32;
        let mut cmd = Command::new("mpv");
        cmd.args([
            "--no-video",
            "--no-terminal",
            "--really-quiet",
            "--audio-display=no",
            "--force-window=no",
            "--cache=yes",
            "--cache-secs=8",
            "--network-timeout=20",
            "--user-agent=Mozilla/5.0 (X11; Linux aarch64) LiveRadio/1.0",
            &format!("--volume={volume}"),
        ]);

        #[cfg(unix)]
        {
            let path = std::env::temp_dir().join(format!("live-radio-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            cmd.arg(format!("--input-ipc-server={}", path.display()));
            self.socket = Some(path);
        }

        cmd.arg(&url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        match cmd.spawn() {
            Ok(child) => {
                self.child = Some(child);
                self.started = Instant::now();
                self.status = "Connecting…".to_string();
            }
            Err(_) => {
                self.failed = true;
                self.status = "mpv is not installed, so audio cannot start".to_string();
                #[cfg(unix)]
                {
                    if let Some(path) = &self.socket {
                        let _ = std::fs::remove_file(path);
                    }
                    self.socket = None;
                }
            }
        }
    }

    pub fn toggle(&mut self) {
        if self.station.is_none() {
            return;
        }
        if self.child.is_some() {
            if self.paused {
                self.resume();
            } else {
                self.pause();
            }
        } else {
            let station = self.station.clone().unwrap();
            self.play(&station);
        }
    }

    pub fn pause(&mut self) {
        if self.child.is_none() {
            return;
        }
        if self.ipc("{\"command\":[\"set_property\",\"pause\",true]}") {
            self.paused = true;
            self.status = "Paused".to_string();
        } else {
            // No IPC (e.g. Windows): stop the helper and remember the pause.
            self.stop_process();
            self.paused = true;
            self.status = "Paused".to_string();
        }
    }

    pub fn resume(&mut self) {
        if self.child.is_some() {
            if self.ipc("{\"command\":[\"set_property\",\"pause\",false]}") {
                self.paused = false;
                self.status = "Connecting…".to_string();
            }
        } else if let Some(station) = self.station.clone() {
            self.play(&station);
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        let value = (self.volume * 100.0).round() as i32;
        if self.child.is_some() {
            let _ = self.ipc(&format!(
                "{{\"command\":[\"set_property\",\"volume\",{value}]}}"
            ));
        }
    }

    pub fn toggle_mute(&mut self) {
        if self.volume > 0.001 {
            self.last_volume = self.volume;
            self.set_volume(0.0);
        } else {
            let v = if self.last_volume > 0.001 {
                self.last_volume
            } else {
                0.7
            };
            self.set_volume(v);
        }
    }

    /// Called each frame: notices a helper process that has ended.
    pub fn poll(&mut self) {
        let exited = match self.child.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => true,
            },
            None => false,
        };
        if exited {
            self.child = None;
            self.paused = false;
            self.status = "Stream stopped — press play to reconnect".to_string();
            self.failed = true;
            #[cfg(unix)]
            {
                if let Some(path) = &self.socket {
                    let _ = std::fs::remove_file(path);
                }
                self.socket = None;
            }
        } else if self.child.is_some()
            && !self.paused
            && self.status == "Connecting…"
            && self.started.elapsed() > Duration::from_millis(1500)
        {
            self.status = "Live".to_string();
        }
    }

    pub fn shutdown(&mut self) {
        self.stop_process();
        self.station = None;
    }

    fn stop_process(&mut self) {
        if self.child.is_some() {
            let _ = self.ipc("{\"command\":[\"quit\"]}");
        }
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        #[cfg(unix)]
        {
            if let Some(path) = &self.socket {
                let _ = std::fs::remove_file(path);
            }
            self.socket = None;
        }
        self.paused = false;
    }

    fn ipc(&self, _json: &str) -> bool {
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::net::UnixStream;

            if let Some(path) = &self.socket {
                for _ in 0..10 {
                    if let Ok(mut stream) = UnixStream::connect(path) {
                        if stream.write_all(_json.as_bytes()).is_ok()
                            && stream.write_all(b"\n").is_ok()
                        {
                            return true;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(12));
                }
            }
        }
        false
    }
}
