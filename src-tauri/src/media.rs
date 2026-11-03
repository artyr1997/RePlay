//! The Mac's media keys (⏮ ⏯ ⏭ on the top row, and the headphones' buttons): macOS sends them to the app
//! playing now, which says so through Now Playing (MPNowPlayingInfoCenter) and listens for them in
//! MPRemoteCommandCenter. Now Playing also shows the song (or the station) in Control Centre.

use crate::{next, previous, status, toggle_play, AppState};
use block2::RcBlock;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use std::ffi::CString;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {
    static MPMediaItemPropertyTitle: *mut AnyObject;
    static MPMediaItemPropertyArtist: *mut AnyObject;
    static MPMediaItemPropertyPlaybackDuration: *mut AnyObject;
    static MPNowPlayingInfoPropertyElapsedPlaybackTime: *mut AnyObject;
    static MPNowPlayingInfoPropertyPlaybackRate: *mut AnyObject;
}

#[derive(Clone, Copy)]
enum Key {
    Toggle,
    Play,
    Pause,
    Next,
    Previous,
}

/// What a key does, off the main thread (a song starting waits a moment for the audio), then the UI is told.
fn press(app: &AppHandle, key: Key) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let playing = state.audio.status.lock().map(|s| s.playing).unwrap_or(false);
        let _ = match key {
            Key::Toggle => toggle_play(app.clone(), app.state()),
            Key::Play if !playing => toggle_play(app.clone(), app.state()),
            Key::Pause if playing => toggle_play(app.clone(), app.state()),
            Key::Play | Key::Pause => Ok(()),
            Key::Next => next(app.clone(), app.state()),
            Key::Previous => previous(app.clone(), app.state()),
        };
        std::thread::sleep(Duration::from_millis(40));
        if let Ok(s) = status(&state) {
            let _ = app.emit("player", s);
        }
    });
}

fn ns_string(text: &str) -> *mut AnyObject {
    let text = CString::new(text.replace('\0', "")).unwrap_or_default();
    // SAFETY: a plain NSString from a C string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()] }
}

fn ns_number(value: f64) -> *mut AnyObject {
    // SAFETY: a plain NSNumber.
    unsafe { msg_send![class!(NSNumber), numberWithDouble: value] }
}

/// Listens for the media keys. On the main thread, once.
pub fn listen(app: &AppHandle) {
    // SAFETY: MediaPlayer's shared command centre, its commands and their handlers, on the main thread.
    unsafe {
        let center: *mut AnyObject = msg_send![class!(MPRemoteCommandCenter), sharedCommandCenter];
        if center.is_null() {
            return;
        }
        let commands: [(*mut AnyObject, Key); 5] = [
            (msg_send![center, togglePlayPauseCommand], Key::Toggle),
            (msg_send![center, playCommand], Key::Play),
            (msg_send![center, pauseCommand], Key::Pause),
            (msg_send![center, nextTrackCommand], Key::Next),
            (msg_send![center, previousTrackCommand], Key::Previous),
        ];
        for (command, key) in commands {
            if command.is_null() {
                continue;
            }
            let app = app.clone();
            // MPRemoteCommandHandlerStatus: 0 is success
            let handler = RcBlock::new(move |_event: *mut AnyObject| -> isize {
                press(&app, key);
                0
            });
            let _: () = msg_send![command, setEnabled: true];
            let _: *mut AnyObject = msg_send![command, addTargetWithHandler: &*handler];
        }
    }
    // Now Playing kept up to date: macOS only routes the keys to an app that says it is playing (or paused)
    let app = app.clone();
    std::thread::spawn(move || {
        let mut last = String::new();
        loop {
            std::thread::sleep(Duration::from_millis(1000));
            let Ok(s) = status(&app.state::<AppState>()) else { continue };
            let (title, artist, duration) = match (&s.track, &s.station) {
                (Some(t), _) => (t.title.clone(), t.artist.clone(), t.duration_ms as f64 / 1000.0),
                (None, Some(st)) => (s.on_air.clone().unwrap_or_else(|| st.name.clone()), st.name.clone(), 0.0),
                _ => (String::new(), String::new(), 0.0),
            };
            let elapsed = s.position_ms as f64 / 1000.0;
            // the info again when something changed, and every 10 s so the elapsed time stays right
            let now = format!("{title}\u{1}{artist}\u{1}{}\u{1}{}", s.playing, (elapsed / 10.0) as u64);
            if now == last {
                continue;
            }
            last = now;
            let playing = s.playing;
            let has = !title.is_empty();
            let _ = app.run_on_main_thread(move || unsafe {
                // SAFETY: the default Now Playing centre, on the main thread.
                let center: *mut AnyObject = msg_send![class!(MPNowPlayingInfoCenter), defaultCenter];
                if center.is_null() {
                    return;
                }
                if !has {
                    let _: () = msg_send![center, setNowPlayingInfo: std::ptr::null_mut::<AnyObject>()];
                    // MPNowPlayingPlaybackStateStopped
                    let _: () = msg_send![center, setPlaybackState: 3usize];
                    return;
                }
                let info: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
                let _: () = msg_send![info, setObject: ns_string(&title), forKey: MPMediaItemPropertyTitle];
                let _: () = msg_send![info, setObject: ns_string(&artist), forKey: MPMediaItemPropertyArtist];
                if duration > 0.0 {
                    let _: () = msg_send![info, setObject: ns_number(duration), forKey: MPMediaItemPropertyPlaybackDuration];
                    let _: () = msg_send![info, setObject: ns_number(elapsed), forKey: MPNowPlayingInfoPropertyElapsedPlaybackTime];
                }
                let rate = if playing { 1.0 } else { 0.0 };
                let _: () = msg_send![info, setObject: ns_number(rate), forKey: MPNowPlayingInfoPropertyPlaybackRate];
                let _: () = msg_send![center, setNowPlayingInfo: info];
                // MPNowPlayingPlaybackState: playing 1, paused 2
                let _: () = msg_send![center, setPlaybackState: if playing { 1usize } else { 2usize }];
            });
        }
    });
}
