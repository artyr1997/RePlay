//! Playback: a queue (order, shuffle, repeat) and an audio thread that plays files
//! with rodio. The next track starts from Rust itself, so music keeps going even
//! when the window is hidden.

use serde::Serialize;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub struct QueueItem {
    pub id: i64,
    pub path: PathBuf,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// Tiny xorshift so shuffling needs no extra crate.
struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

/// What plays and in which order. `order` holds indexes into `items`;
/// `pos` is where we are in `order`. History stays before `pos`.
pub struct Queue {
    items: Vec<QueueItem>,
    order: Vec<usize>,
    pos: usize,
    pub shuffle: bool,
    pub repeat: Repeat,
    rng: Rng,
}

impl Default for Queue {
    fn default() -> Self {
        Self { items: Vec::new(), order: Vec::new(), pos: 0, shuffle: false, repeat: Repeat::Off, rng: Rng::new() }
    }
}

impl Queue {
    /// Replaces the queue with `items`, starting at `start`.
    pub fn set(&mut self, items: Vec<QueueItem>, start: usize) {
        let start = start.min(items.len().saturating_sub(1));
        self.items = items;
        self.order = (0..self.items.len()).collect();
        if self.shuffle && !self.items.is_empty() {
            // the chosen track plays first, the rest in random order
            self.order.remove(start);
            self.rng.shuffle(&mut self.order);
            self.order.insert(0, start);
            self.pos = 0;
        } else {
            self.pos = start;
        }
    }

    pub fn current(&self) -> Option<&QueueItem> {
        self.order.get(self.pos).map(|&i| &self.items[i])
    }

    /// Moves on. `auto` = the track finished by itself (repeat-one replays it).
    pub fn advance(&mut self, auto: bool) -> Option<&QueueItem> {
        if self.order.is_empty() {
            return None;
        }
        if auto && self.repeat == Repeat::One {
            return self.current();
        }
        if self.pos + 1 < self.order.len() {
            self.pos += 1;
        } else if self.repeat != Repeat::Off {
            self.pos = 0;
            if self.shuffle {
                self.rng.shuffle(&mut self.order);
            }
        } else {
            return None;
        }
        self.current()
    }

    pub fn back(&mut self) -> Option<&QueueItem> {
        self.pos = self.pos.saturating_sub(1);
        self.current()
    }

    /// Shuffles what comes next (history and the current track stay put), or restores album order.
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if self.order.is_empty() {
            return;
        }
        let current = self.order[self.pos];
        if on {
            let mut upcoming = self.order.split_off(self.pos + 1);
            self.rng.shuffle(&mut upcoming);
            self.order.extend(upcoming);
        } else {
            self.order = (0..self.items.len()).collect();
            self.pos = current;
        }
    }

    pub fn cycle_repeat(&mut self) -> Repeat {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
        self.repeat
    }

    /// Puts tracks right after the current one.
    pub fn play_next(&mut self, items: Vec<QueueItem>) {
        let first = self.items.len();
        let count = items.len();
        self.items.extend(items);
        let at = (self.pos + 1).min(self.order.len());
        self.order.splice(at..at, first..first + count);
    }

    /// Puts tracks at the end.
    pub fn append(&mut self, items: Vec<QueueItem>) {
        let first = self.items.len();
        let count = items.len();
        self.items.extend(items);
        self.order.extend(first..first + count);
    }

    /// Track ids still to come, in play order.
    pub fn upcoming(&self) -> Vec<i64> {
        self.order.iter().skip(self.pos + 1).map(|&i| self.items[i].id).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

// ── audio thread ──

enum Cmd {
    Load { path: PathBuf, autoplay: bool },
    Play,
    Pause,
    Seek(Duration),
    Stop,
}

/// What the UI shows; updated by the audio thread several times a second.
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AudioStatus {
    pub playing: bool,
    pub position_ms: u64,
    pub error: Option<String>,
}

pub struct Audio {
    tx: Sender<Cmd>,
    pub status: Arc<Mutex<AudioStatus>>,
}

fn decode(path: &PathBuf) -> Result<rodio::Decoder<BufReader<File>>, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let len = file.metadata().map_err(|e| e.to_string())?.len();
    let mut builder = rodio::Decoder::builder()
        .with_data(BufReader::new(file))
        .with_byte_len(len)
        .with_seekable(true)
        .with_gapless(true);
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        builder = builder.with_hint(ext);
    }
    builder.build().map_err(|e| e.to_string())
}

impl Audio {
    /// Starts the audio thread. `on_end` runs (on that thread) when a track finishes by itself.
    pub fn spawn(on_end: impl Fn() + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(AudioStatus::default()));
        let shared = status.clone();
        std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || run(rx, shared, on_end))
            .expect("audio thread");
        Self { tx, status }
    }

    pub fn load(&self, path: PathBuf, autoplay: bool) {
        let _ = self.tx.send(Cmd::Load { path, autoplay });
    }
    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn seek(&self, to: Duration) {
        let _ = self.tx.send(Cmd::Seek(to));
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Cmd::Stop);
    }
}

fn run(rx: Receiver<Cmd>, status: Arc<Mutex<AudioStatus>>, on_end: impl Fn()) {
    let set = |f: &dyn Fn(&mut AudioStatus)| {
        if let Ok(mut s) = status.lock() {
            f(&mut s);
        }
    };
    let mut device = match rodio::DeviceSinkBuilder::open_default_sink() {
        Ok(d) => d,
        Err(e) => {
            set(&|s| s.error = Some(format!("No audio output: {e}")));
            return;
        }
    };
    device.log_on_drop(false);
    let player = rodio::Player::connect_new(device.mixer());
    // a track is loaded and hasn't ended yet
    let mut loaded = false;

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Cmd::Load { path, autoplay }) => {
                player.clear();
                match decode(&path) {
                    Ok(source) => {
                        player.append(source);
                        if autoplay {
                            player.play();
                        }
                        loaded = true;
                        set(&|s| s.error = None);
                    }
                    Err(e) => {
                        loaded = false;
                        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        set(&|s| s.error = Some(format!("Can't play “{name}”: {e}")));
                    }
                }
            }
            Ok(Cmd::Play) => player.play(),
            Ok(Cmd::Pause) => player.pause(),
            Ok(Cmd::Seek(to)) => {
                let _ = player.try_seek(to);
            }
            Ok(Cmd::Stop) => {
                player.clear();
                loaded = false;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }

        let finished = loaded && !player.is_paused() && player.empty();
        set(&|s| {
            s.playing = loaded && !player.is_paused() && !player.empty();
            s.position_ms = if loaded { player.get_pos().as_millis() as u64 } else { 0 };
        });
        if finished {
            loaded = false;
            on_end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(n: i64) -> Vec<QueueItem> {
        (0..n).map(|id| QueueItem { id, path: PathBuf::from(format!("/m/{id}.mp3")) }).collect()
    }

    fn id(q: &Queue) -> Option<i64> {
        q.current().map(|i| i.id)
    }

    #[test]
    fn plays_in_order_and_stops_at_the_end() {
        let mut q = Queue::default();
        q.set(items(3), 1);
        assert_eq!(id(&q), Some(1));
        assert_eq!(q.advance(true).map(|i| i.id), Some(2));
        assert_eq!(q.advance(true), None, "repeat off stops after the last track");
        assert_eq!(q.back().map(|i| i.id), Some(1));
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_replays() {
        let mut q = Queue::default();
        q.set(items(2), 1);
        q.cycle_repeat(); // all
        assert_eq!(q.advance(true).map(|i| i.id), Some(0));
        q.cycle_repeat(); // one
        assert_eq!(q.advance(true).map(|i| i.id), Some(0), "finished by itself → same track");
        assert_eq!(q.advance(false).map(|i| i.id), Some(1), "NEXT still moves on");
        assert_eq!(q.cycle_repeat(), Repeat::Off);
    }

    #[test]
    fn shuffle_keeps_the_current_track_and_plays_everything_once() {
        let mut q = Queue::default();
        q.set_shuffle(true);
        q.set(items(20), 7);
        assert_eq!(id(&q), Some(7), "chosen track plays first");
        let mut seen = vec![7];
        while let Some(i) = q.advance(true) {
            seen.push(i.id);
        }
        seen.sort();
        assert_eq!(seen, (0..20).collect::<Vec<_>>());

        q.set(items(20), 7);
        q.advance(false);
        let playing = id(&q);
        q.set_shuffle(false);
        assert_eq!(id(&q), playing, "turning shuffle off keeps the current track");
    }

    /// One second of a 440 Hz tone as 16-bit mono WAV.
    pub fn wav_bytes() -> Vec<u8> {
        let rate = 44_100u32;
        let samples: Vec<i16> = (0..rate).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16).collect();
        let data_len = (samples.len() * 2) as u32;
        let mut v = Vec::new();
        v.extend_from_slice(b"RIFF");
        v.extend_from_slice(&(36 + data_len).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes()); // PCM
        v.extend_from_slice(&1u16.to_le_bytes()); // mono
        v.extend_from_slice(&rate.to_le_bytes());
        v.extend_from_slice(&(rate * 2).to_le_bytes());
        v.extend_from_slice(&2u16.to_le_bytes());
        v.extend_from_slice(&16u16.to_le_bytes());
        v.extend_from_slice(b"data");
        v.extend_from_slice(&data_len.to_le_bytes());
        for s in samples {
            v.extend_from_slice(&s.to_le_bytes());
        }
        v
    }

    #[test]
    fn decodes_real_audio_and_reads_its_length() {
        let path = std::env::temp_dir().join(format!("resound-tone-{}.wav", std::process::id()));
        std::fs::write(&path, wav_bytes()).unwrap();

        let source = decode(&path).expect("rodio decodes the file");
        let frames = source.count();
        assert!(frames >= 44_000, "about one second of samples, got {frames}");

        let info = crate::scan::read_info(&path);
        assert!((990..=1010).contains(&info.duration_ms), "lofty reads the length: {}", info.duration_ms);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn play_next_and_append() {
        let mut q = Queue::default();
        q.set(items(3), 0);
        q.play_next(vec![QueueItem { id: 10, path: "/m/10.mp3".into() }]);
        q.append(vec![QueueItem { id: 11, path: "/m/11.mp3".into() }]);
        assert_eq!(q.upcoming(), vec![10, 1, 2, 11]);
    }
}
