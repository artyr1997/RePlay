//! Internet radio: stations from Radio Browser (radio-browser.info, an open, free list of the world's
//! stations), and their streams read for the audio thread, with the song on air taken out of them
//! (ICY metadata, which most stations send).

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::Mutex;
use std::time::Duration;

const API: &str = "https://all.api.radio-browser.info/json";
const AGENT: &str = concat!("ReSound/", env!("CARGO_PKG_VERSION"));

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Station {
    pub uuid: String,
    pub name: String,
    pub url: String,
    pub country: String,
    pub codec: String,
    pub bitrate: u32,
}

/// What the left column lists: a name, and how Radio Browser finds its stations.
#[derive(Serialize, Clone, Debug)]
pub struct Genre {
    pub name: &'static str,
    key: &'static str,
}

/// A short list, quieter than the most used tags (which are mostly countries and words like "fm");
/// Japan and Ukraine by country, the rest by tag.
pub const GENRES: &[Genre] = &[
    Genre { name: "Jazz", key: "tag:jazz" },
    Genre { name: "Smooth Jazz", key: "tag:smooth jazz" },
    Genre { name: "Lo-fi", key: "tag:lofi" },
    Genre { name: "Ambient", key: "tag:ambient" },
    Genre { name: "Chillout", key: "tag:chillout" },
    Genre { name: "Classical", key: "tag:classical" },
    Genre { name: "Piano", key: "tag:piano" },
    Genre { name: "Soundtrack", key: "tag:soundtrack" },
    Genre { name: "Meditation", key: "tag:meditation" },
    Genre { name: "Electronic", key: "tag:electronic" },
    Genre { name: "House", key: "tag:house" },
    Genre { name: "Indie", key: "tag:indie" },
    Genre { name: "Rock", key: "tag:rock" },
    Genre { name: "Classic Rock", key: "tag:classic rock" },
    Genre { name: "Pop", key: "tag:pop" },
    Genre { name: "80s", key: "tag:80s" },
    Genre { name: "90s", key: "tag:90s" },
    Genre { name: "Oldies", key: "tag:oldies" },
    Genre { name: "Soul", key: "tag:soul" },
    Genre { name: "Blues", key: "tag:blues" },
    Genre { name: "Folk", key: "tag:folk" },
    Genre { name: "Reggae", key: "tag:reggae" },
    Genre { name: "Hip Hop", key: "tag:hip hop" },
    Genre { name: "Japan", key: "country:JP" },
    Genre { name: "Ukraine", key: "country:UA" },
];

/// Radio Browser's answer, of which ReSound keeps a little.
#[derive(Deserialize)]
struct Listed {
    stationuuid: String,
    name: String,
    url_resolved: String,
    #[serde(default)]
    country: String,
    #[serde(default)]
    codec: String,
    #[serde(default)]
    bitrate: u32,
    #[serde(default)]
    hls: u8,
}

/// What the audio thread can play: a plain stream (not HLS) in a format it decodes. Ogg may hold
/// Vorbis (played) or Opus (not): the list doesn't say, the station's name or address often does.
/// Nothing under 64 kbps either (thin sound; "lo-fi" stations are sometimes only that).
fn playable(s: &Listed) -> bool {
    let codec = s.codec.to_uppercase();
    let opus = codec == "OGG" && format!("{} {}", s.name, s.url_resolved).to_lowercase().contains("opus");
    let thin = s.bitrate > 0 && s.bitrate < 64;
    s.hls == 0 && s.url_resolved.starts_with("http") && !opus && !thin && matches!(codec.as_str(), "MP3" | "AAC" | "AAC+" | "OGG" | "FLAC")
}

fn get(path: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(AGENT)
        .build()
        .into();
    agent
        .get(&format!("{API}{path}"))
        .call()
        .map_err(|e| format!("Radio Browser didn't answer: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

fn stations_at(path: &str) -> Result<Vec<Station>, String> {
    let listed: Vec<Listed> = serde_json::from_str(&get(path)?).map_err(|e| e.to_string())?;
    let mut seen = std::collections::HashSet::new();
    Ok(listed
        .into_iter()
        .filter(playable)
        // the same station is often listed more than once
        .filter(|s| seen.insert(s.name.trim().to_lowercase()))
        .map(|s| Station {
            uuid: s.stationuuid,
            name: s.name.trim().to_string(),
            url: s.url_resolved,
            country: s.country,
            codec: s.codec,
            bitrate: s.bitrate,
        })
        .collect())
}

fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

const ORDER: &str = "order=votes&reverse=true&hidebroken=true&limit=200";

/// A genre's stations, the most liked first.
pub fn stations(genre: &str) -> Result<Vec<Station>, String> {
    let key = GENRES.iter().find(|g| g.name == genre).map(|g| g.key).ok_or("No such genre")?;
    match key.split_once(':') {
        Some(("country", code)) => stations_at(&format!("/stations/bycountrycodeexact/{code}?{ORDER}")),
        Some((_, tag)) => stations_at(&format!("/stations/bytagexact/{}?{ORDER}", encode(tag))),
        None => Ok(Vec::new()),
    }
}

/// Stations whose name has `query` in it.
pub fn search(query: &str) -> Result<Vec<Station>, String> {
    stations_at(&format!("/stations/search?name={}&{ORDER}", encode(query.trim())))
}

/// Radio Browser counts what is listened to (its way of ranking stations); asked for, as it asks.
pub fn count_click(uuid: &str) {
    let path = format!("/url/{uuid}");
    std::thread::spawn(move || {
        let _ = get(&path);
    });
}

// ── the stream ──

/// A station's sound as the decoder reads it: what the network brings, without the song titles
/// mixed into it. It can't go back, except over its first bytes (kept while the decoder works out
/// the format, which may try one and go back to the start).
pub struct Stream {
    chunks: Mutex<Receiver<Vec<u8>>>,
    head: Vec<u8>,
    tail: Vec<u8>,
    at_tail: usize,
    pos: u64,
}

const HEAD: usize = 256 * 1024;

impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let pos = self.pos as usize;
        // still within what was kept from the start
        if pos < self.head.len() {
            let n = (self.head.len() - pos).min(buf.len());
            buf[..n].copy_from_slice(&self.head[pos..pos + n]);
            self.pos += n as u64;
            return Ok(n);
        }
        if self.at_tail >= self.tail.len() {
            let next = self.chunks.lock().map_err(|_| io::Error::other("stream"))?.recv();
            match next {
                Ok(chunk) => {
                    self.tail = chunk;
                    self.at_tail = 0;
                }
                // the station stopped, or the network did
                Err(_) => return Ok(0),
            }
        }
        let n = (self.tail.len() - self.at_tail).min(buf.len());
        buf[..n].copy_from_slice(&self.tail[self.at_tail..self.at_tail + n]);
        if self.head.len() < HEAD && pos == self.head.len() {
            self.head.extend_from_slice(&self.tail[self.at_tail..self.at_tail + n]);
        }
        self.at_tail += n;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Stream {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let target = match to {
            SeekFrom::Start(n) => n as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(_) => return Err(io::Error::new(io::ErrorKind::Unsupported, "a live stream has no end")),
        };
        // to where it is, or back over the kept start while nothing has been read past it (the bytes
        // after the start then are the tail's unread ones)
        let within = target >= 0 && target as usize <= self.head.len() && self.pos as usize <= self.head.len();
        if target != self.pos as i64 && !within {
            return Err(io::Error::new(io::ErrorKind::Unsupported, "a live stream can't skip"));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

/// The song on air, as the station's metadata names it: `StreamTitle='Artist - Song';`
fn stream_title(meta: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(meta);
    let start = text.find("StreamTitle='")? + "StreamTitle='".len();
    let end = text[start..].find("';").map(|e| start + e).unwrap_or(text.len());
    let title: String = text[start..end].chars().filter(|&c| c != '\0' && c != '\u{feff}').collect();
    let title = title.trim().to_string();
    (!title.is_empty()).then_some(title)
}

/// Splits the station's bytes into sound (sent on) and, every `every` bytes, a block of metadata.
fn pump(mut body: impl Read, every: usize, send: SyncSender<Vec<u8>>, on_title: impl Fn(Option<String>)) {
    let mut left = every;
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        let want = if every > 0 { left.min(buf.len()) } else { buf.len() };
        let n = match body.read(&mut buf[..want]) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        // the player is gone (another station, a pause): stop reading
        if send.send(buf[..n].to_vec()).is_err() {
            return;
        }
        if every == 0 {
            continue;
        }
        left -= n;
        if left == 0 {
            let mut size = [0u8; 1];
            if body.read_exact(&mut size).is_err() {
                return;
            }
            let mut meta = vec![0u8; size[0] as usize * 16];
            if body.read_exact(&mut meta).is_err() {
                return;
            }
            if !meta.is_empty() {
                on_title(stream_title(&meta));
            }
            left = every;
        }
    }
}

/// Connects to a station: its stream, the decoder's hint for the format, and the song titles to
/// `on_title` as they come.
pub fn open(url: &str, on_title: impl Fn(Option<String>) + Send + 'static) -> Result<(Stream, Option<&'static str>), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_recv_response(Some(Duration::from_secs(10)))
        .user_agent(AGENT)
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Icy-MetaData", "1")
        .call()
        .map_err(|e| format!("The station didn't answer: {e}"))?;
    let every = response
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let kind = response.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    let hint = if kind.contains("mpeg") || kind.contains("mp3") {
        Some("mp3")
    } else if kind.contains("aac") || kind.contains("mp4") {
        Some("aac")
    } else if kind.contains("ogg") || kind.contains("opus") || kind.contains("vorbis") {
        Some("ogg")
    } else if kind.contains("flac") {
        Some("flac")
    } else {
        None
    };
    let body = response.into_body().into_reader();
    // a few seconds of sound at most in waiting: the network can't run far ahead of the speakers
    let (send, chunks) = mpsc::sync_channel(64);
    std::thread::Builder::new()
        .name("radio".into())
        .spawn(move || pump(body, every, send, on_title))
        .map_err(|e| e.to_string())?;
    Ok((Stream { chunks: Mutex::new(chunks), head: Vec::new(), tail: Vec::new(), at_tail: 0, pos: 0 }, hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn reads_the_song_on_air() {
        assert_eq!(stream_title(b"StreamTitle='Miles Davis - So What';StreamUrl='';\0\0"), Some("Miles Davis - So What".into()));
        assert_eq!(stream_title(b"StreamTitle='';\0\0\0"), None);
    }

    #[test]
    fn takes_the_titles_out_of_the_sound() {
        // 4 bytes of sound, then a block of metadata (1 × 16 bytes), then 4 more bytes of sound
        let mut data = b"abcd".to_vec();
        data.push(1);
        let mut meta = b"StreamTitle='X';".to_vec();
        meta.resize(16, 0);
        data.extend(meta);
        data.extend(b"efgh");
        let (send, chunks) = mpsc::sync_channel(8);
        let titles = Arc::new(Mutex::new(Vec::new()));
        let seen = titles.clone();
        pump(&data[..], 4, send, move |t| seen.lock().unwrap().push(t));
        let sound: Vec<u8> = chunks.try_iter().flatten().collect();
        assert_eq!(sound, b"abcdefgh");
        assert_eq!(*titles.lock().unwrap(), vec![Some("X".to_string())]);
    }

    /// Asks Radio Browser for a few genres' stations and decodes a few seconds of the first ones that answer
    /// (no sound: the samples are only counted). Needs the network: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn tunes_in_to_real_stations() {
        for genre in ["Jazz", "Lo-fi", "Ukraine"] {
            let list = stations(genre).expect("Radio Browser answers");
            assert!(list.len() > 5, "{genre} stations: {}", list.len());
            let played = play_a_few(&list);
            assert!(played >= 3, "{genre}: stations that played: {played}");
        }
    }

    fn play_a_few(list: &[Station]) -> usize {
        let mut played = 0;
        for station in list.iter().take(6) {
            let titles = Arc::new(Mutex::new(None));
            let seen = titles.clone();
            let Ok((stream, hint)) = open(&station.url, move |t| *seen.lock().unwrap() = t) else { continue };
            let mut builder = rodio::Decoder::builder().with_data(stream).with_seekable(false);
            if let Some(hint) = hint {
                builder = builder.with_hint(hint);
            }
            let Ok(decoder) = builder.build() else { continue };
            let samples = decoder.take(44_100 * 2 * 3).count();
            println!("{} ({} {}): {samples} samples, on air: {:?}", station.name, station.codec, station.bitrate, titles.lock().unwrap());
            if samples > 44_100 {
                played += 1;
            }
        }
        played
    }

    #[test]
    fn goes_back_over_its_start_only() {
        let (send, chunks) = mpsc::sync_channel(8);
        send.send(b"hello ".to_vec()).unwrap();
        send.send(b"world".to_vec()).unwrap();
        drop(send);
        let mut s = Stream { chunks: Mutex::new(chunks), head: Vec::new(), tail: Vec::new(), at_tail: 0, pos: 0 };
        let mut first = [0u8; 4];
        s.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"hell");
        s.seek(SeekFrom::Start(0)).unwrap();
        let mut all = String::new();
        s.read_to_string(&mut all).unwrap();
        assert_eq!(all, "hello world");
        assert!(s.seek(SeekFrom::End(0)).is_err());
    }
}
