//! Live catalogue access.
//!
//! The public iHeart Radio endpoints answer without an account. Requests are
//! made with the `curl` command so that no TLS stack has to be compiled into
//! the app, and so the same code path works on Windows and on the Pi.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const API: &str = "https://us.api.iheart.com/api/v2/content";
const UA: &str =
    "Mozilla/5.0 (X11; Linux aarch64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36";

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Genre {
    pub id: u64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Station {
    pub id: u64,
    pub name: String,
    pub call: String,
    pub desc: String,
    pub freq: String,
    pub genre: String,
    pub market: String,
    pub cume: u64,
    /// Preferred adaptive stream (https HLS).
    pub hls: String,
    /// Fallback progressive stream; also used for the ICY title probe.
    pub shoutcast: String,
    /// Square station logo (256px), empty when the service has none.
    pub logo: String,
}

impl Station {
    pub fn subtitle(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.freq.is_empty() {
            let band = if self.freq.contains('.') { "FM" } else { "AM" };
            parts.push(format!("{} {}", self.freq, band));
        }
        if !self.market.is_empty() {
            parts.push(self.market.clone());
        }
        if parts.is_empty() {
            self.genre.clone()
        } else {
            parts.join(" · ")
        }
    }

    pub fn stream(&self) -> &str {
        if !self.hls.is_empty() {
            &self.hls
        } else {
            &self.shoutcast
        }
    }
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run curl: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        let why = why.lines().last().unwrap_or("network error").trim();
        return Err(format!("could not reach the station service ({why})"));
    }
    Ok(out.stdout)
}

fn get_json(url: &str) -> Result<Value, String> {
    let body = curl(&[
        "-sS",
        "-L",
        "--compressed",
        "--max-time",
        "30",
        "--retry",
        "1",
        "-A",
        UA,
        url,
    ])?;
    serde_json::from_slice(&body).map_err(|_| "the station service sent an unreadable reply".to_string())
}

fn text(v: &Value, key: &str) -> String {
    match v.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn stream_of(streams: &Value, key: &str) -> String {
    streams
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn parse_station(v: &Value) -> Option<Station> {
    let streams = v.get("streams")?;
    if matches!(v.get("isActive"), Some(Value::Bool(false))) {
        return None;
    }

    let hls = [
        stream_of(streams, "secure_hls_stream"),
        stream_of(streams, "hls_stream"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_default();
    let shoutcast = [
        stream_of(streams, "secure_shoutcast_stream"),
        stream_of(streams, "shoutcast_stream"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or_default();

    if hls.is_empty() && shoutcast.is_empty() {
        return None;
    }

    let genre = v
        .get("genres")
        .and_then(Value::as_array)
        .and_then(|g| g.first())
        .map(|g| text(g, "name"))
        .unwrap_or_default();

    let market = v
        .get("markets")
        .and_then(Value::as_array)
        .and_then(|m| {
            m.iter()
                .find(|m| matches!(m.get("primary"), Some(Value::Bool(true))))
                .or_else(|| m.first())
        })
        .map(|m| {
            let city = text(m, "city");
            let state = text(m, "stateAbbreviation");
            match (city.is_empty(), state.is_empty()) {
                (false, false) => format!("{city}, {state}"),
                (false, true) => city,
                (true, false) => state,
                (true, true) => String::new(),
            }
        })
        .unwrap_or_default();

    let cume = v.get("cume").and_then(Value::as_u64).unwrap_or(0);

    let id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
    let name = text(v, "name");
    if id == 0 || name.is_empty() {
        return None;
    }

    Some(Station {
        id,
        name,
        call: text(v, "callLetters"),
        desc: text(v, "description"),
        freq: text(v, "freq"),
        genre,
        market,
        cume,
        hls,
        shoutcast,
        logo: logo_url(&text(v, "logo")),
    })
}

/// Ask the image service for a plain 256px square instead of the default
/// branded variant the catalogue links to.
fn logo_url(raw: &str) -> String {
    let base = raw.split('?').next().unwrap_or("").trim();
    if base.is_empty() {
        return String::new();
    }
    if base.starts_with("https://i.iheart.com/") {
        format!("{base}?ops=fit(256%2C256)")
    } else {
        raw.to_string()
    }
}

/// Download an image (logo) with curl.
pub fn fetch_image(url: &str) -> Result<Vec<u8>, String> {
    let body = curl(&["-sS", "-L", "--fail", "--max-time", "20", "-A", UA, url])?;
    if body.is_empty() {
        return Err("empty image".to_string());
    }
    Ok(body)
}

pub fn fetch_genres() -> Result<Vec<Genre>, String> {
    let value = get_json(&format!("{API}/genre"))?;
    let hits = value
        .get("hits")
        .and_then(Value::as_array)
        .ok_or("the station service sent an unreadable genre list")?;
    let mut out: Vec<Genre> = hits
        .iter()
        .filter(|g| !matches!(g.get("display"), Some(Value::Bool(false))))
        .filter_map(|g| {
            Some(Genre {
                id: g.get("id").and_then(Value::as_u64)?,
                name: text(g, "name"),
            })
        })
        .filter(|g| !g.name.is_empty())
        .collect();
    out.sort_by_key(|g| g.name.to_lowercase());
    out.dedup_by_key(|g| g.id);
    Ok(out)
}

pub fn fetch_stations(genre: Option<u64>, limit: usize) -> Result<Vec<Station>, String> {
    let mut url = format!("{API}/liveStations?limit={limit}");
    if let Some(id) = genre {
        url.push_str(&format!("&genreId={id}"));
    }
    let value = get_json(&url)?;
    let hits = value
        .get("hits")
        .and_then(Value::as_array)
        .ok_or("the station service sent an unreadable station list")?;
    let mut out: Vec<Station> = hits.iter().filter_map(parse_station).collect();
    // Keep the service's popularity order but never show a station twice.
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert(s.id));
    Ok(out)
}

pub struct Bootstrap {
    pub genres: Vec<Genre>,
    pub stations: Vec<Station>,
}

pub fn bootstrap() -> Result<Bootstrap, String> {
    let genres = fetch_genres()?;
    let stations = fetch_stations(None, 240)?;
    if stations.is_empty() {
        return Err("the station service returned an empty list".to_string());
    }
    Ok(Bootstrap { genres, stations })
}

fn get_text(url: &str) -> Result<String, String> {
    let body = curl(&[
        "-sS",
        "-L",
        "--compressed",
        "--max-time",
        "20",
        "-A",
        UA,
        url,
    ])?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn playlist_attr(line: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=\"");
    let start = line.find(&needle)? + needle.len();
    let rest = &line[start..];

    let mut value = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => {
                let value = value
                    .replace("&amp;", "&")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'");
                let value = value.trim().to_string();
                return if value.is_empty() { None } else { Some(value) };
            }
            _ => value.push(ch),
        }
    }
    None
}

/// Read the song/artist labels that iHeart embeds in its HLS media playlist.
/// This gives a cleaner "Artist — Title" than the ICY stream title.
pub fn probe_hls(master: &str) -> Option<String> {
    if master.is_empty() || !master.contains(".m3u8") {
        return None;
    }
    let listing = get_text(master).ok()?;
    let variant = listing
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("http") && line.contains(".m3u8"))?;
    let media = get_text(variant).ok()?;

    for line in media.lines() {
        let line = line.trim();
        if !line.starts_with("#EXTINF") {
            continue;
        }
        let title = playlist_attr(line, "title");
        let artist = playlist_attr(line, "artist");
        match (artist, title) {
            (Some(artist), Some(title)) => return Some(format!("{artist} — {title}")),
            (None, Some(title)) => return Some(title),
            _ => {}
        }
    }
    None
}

/// Prefer the HLS labels; fall back to the progressive stream's ICY title.
pub fn probe_metadata(station: &Station) -> Option<String> {
    probe_hls(&station.hls).or_else(|| probe_icy(&station.shoutcast))
}

/// Ask the progressive stream for its ICY title. The probe stops as soon as a
/// title is visible, so it never downloads more than a few seconds of audio.
pub fn probe_icy(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    let mut child = Command::new("curl")
        .args([
            "-sL",
            "-N",
            "--max-time",
            "25",
            "-H",
            "Icy-MetaData: 1",
            "-A",
            UA,
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let mut buf: Vec<u8> = Vec::with_capacity(196_608);
    let mut chunk = [0u8; 8192];
    let deadline = Instant::now() + Duration::from_secs(24);
    let mut found = None;

    while Instant::now() < deadline && buf.len() < 1_200_000 {
        match stdout.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(title) = find_title(&buf) {
                    found = Some(title);
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    found
}

fn find_title(buf: &[u8]) -> Option<String> {
    const NEEDLE: &[u8] = b"StreamTitle='";
    let start = buf
        .windows(NEEDLE.len())
        .position(|w| w == NEEDLE)?
        + NEEDLE.len();
    let rest = &buf[start..];
    let end = rest
        .iter()
        .position(|&b| b == b'\'' || b == b'\0')
        .unwrap_or(rest.len().min(256));
    let raw = &rest[..end.min(256)];
    let title = clean_title(&String::from_utf8_lossy(raw));
    if title.is_empty() || title.eq_ignore_ascii_case("unknown") {
        None
    } else {
        Some(title)
    }
}

/// iHeart's progressive streams put a small key/value payload in StreamTitle,
/// e.g. `- text="WZTU - Artist - Song" song_spot="T" ...`. Keep the readable
/// part; leave ordinary `Artist - Song` titles alone.
fn clean_title(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(start) = raw.find("text=\"") {
        let rest = &raw[start + 6..];
        if let Some(end) = rest.find('"') {
            return rest[..end].trim().to_string();
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::find_title;

    #[test]
    fn title_inside_metadata_block() {
        let mut block = Vec::new();
        block.extend_from_slice(&[0u8; 40]);
        block.extend_from_slice(b"StreamTitle='Artist - Song';");
        block.extend_from_slice(&[0u8; 8]);
        assert_eq!(find_title(&block).as_deref(), Some("Artist - Song"));
    }

    #[test]
    fn missing_title() {
        assert_eq!(find_title(b"nothing here"), None);
    }

    #[test]
    fn iheart_payload_keeps_readable_part() {
        let mut block = Vec::new();
        block.extend_from_slice(
            b"StreamTitle='- text=\"WZTU - Artist - Song\" song_spot=\"T\" spotInstanceId=\"-1\"';",
        );
        assert_eq!(find_title(&block).as_deref(), Some("WZTU - Artist - Song"));
    }

    #[test]
    fn hls_attrs() {
        let line = r#"#EXTINF:10,title="Song \"Name\"" artist="The &amp; Band""#;
        assert_eq!(super::playlist_attr(line, "title").as_deref(), Some("Song \"Name\""));
        assert_eq!(super::playlist_attr(line, "artist").as_deref(), Some("The & Band"));
    }
}



