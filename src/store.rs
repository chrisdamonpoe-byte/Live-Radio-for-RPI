//! Favourites, recently played stations and volume.
//!
//! Stored as one small JSON file under the user's config directory:
//!   Linux:   $XDG_CONFIG_HOME/live-radio/state.json (or ~/.config/live-radio)
//!   Windows: %APPDATA%\LiveRadio\state.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::api::Station;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub favorites: Vec<Station>,
    #[serde(default)]
    pub recent: Vec<Station>,
    #[serde(default = "default_volume")]
    pub volume: f32,
}

fn default_volume() -> f32 {
    0.8
}

impl Default for State {
    fn default() -> Self {
        Self {
            favorites: Vec::new(),
            recent: Vec::new(),
            volume: default_volume(),
        }
    }
}

pub struct Store {
    pub state: State,
    path: PathBuf,
}

impl Store {
    pub fn load() -> Self {
        let path = config_dir().join("state.json");
        let mut state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<State>(&text).ok())
            .unwrap_or_default();
        if !(0.0..=1.0).contains(&state.volume) {
            state.volume = default_volume();
        }
        state.favorites.retain(|s| !s.name.is_empty());
        state.recent.retain(|s| !s.name.is_empty());
        Self { state, path }
    }

    pub fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(&self.state) {
            let _ = std::fs::write(&self.path, text);
        }
    }

    pub fn is_favorite(&self, id: u64) -> bool {
        self.state.favorites.iter().any(|s| s.id == id)
    }

    /// Returns true when the station is now a favourite.
    pub fn toggle_favorite(&mut self, station: &Station) -> bool {
        let added = if self.is_favorite(station.id) {
            self.state.favorites.retain(|s| s.id != station.id);
            false
        } else {
            self.state.favorites.insert(0, station.clone());
            true
        };
        self.save();
        added
    }

    pub fn remember(&mut self, station: &Station) {
        self.state.recent.retain(|s| s.id != station.id);
        self.state.recent.insert(0, station.clone());
        self.state.recent.truncate(12);
        self.save();
    }

    /// Merge fresh stream URLs into saved records so old entries keep working.
    pub fn refresh_saved(&mut self, catalog: &[Station]) -> bool {
        let mut changed = false;
        for saved in self.state.favorites.iter_mut().chain(self.state.recent.iter_mut()) {
            if let Some(fresh) = catalog.iter().find(|s| s.id == saved.id) {
                if fresh.hls != saved.hls || fresh.shoutcast != saved.shoutcast {
                    saved.hls = fresh.hls.clone();
                    saved.shoutcast = fresh.shoutcast.clone();
                    changed = true;
                }
                if fresh.logo != saved.logo {
                    saved.logo = fresh.logo.clone();
                    changed = true;
                }
                if fresh.name != saved.name {
                    saved.name = fresh.name.clone();
                    changed = true;
                }
            }
        }
        if changed {
            self.save();
        }
        changed
    }

    pub fn set_volume_in_memory(&mut self, volume: f32) {
        self.state.volume = volume.clamp(0.0, 1.0);
    }

    pub fn save_volume(&mut self) {
        self.save();
    }
}

fn config_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        return PathBuf::from(appdata).join("LiveRadio");
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("live-radio");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".config/live-radio")
}

/// Where downloaded station logos are kept between runs.
pub fn cache_dir() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("LiveRadio").join("cache");
    }
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("live-radio");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache/live-radio")
}
