//! ReSound: a quiet music player in the ReNote style.
//! Rust scans the music folder, keeps the library and playlists in SQLite and
//! plays the audio; the Svelte UI talks to it through these commands and the
//! `player` / `scan` events.

mod db;
mod player;
mod scan;

use db::{Db, Playlist, Track};
use player::{Audio, QueueItem, Queue, Repeat};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    db: Mutex<Db>,
    db_path: PathBuf,
    queue: Mutex<Queue>,
    audio: Audio,
    scanning: Mutex<bool>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Everything the player bar shows.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Status {
    track: Option<Track>,
    playing: bool,
    position_ms: u64,
    shuffle: bool,
    repeat: Repeat,
    error: Option<String>,
}

fn status(state: &AppState) -> CmdResult<Status> {
    let audio = state.audio.status.lock().map_err(err)?.clone();
    let queue = state.queue.lock().map_err(err)?;
    let track = match queue.current() {
        Some(item) => state.db.lock().map_err(err)?.track(item.id).ok(),
        None => None,
    };
    Ok(Status {
        track,
        playing: audio.playing,
        position_ms: audio.position_ms,
        shuffle: queue.shuffle,
        repeat: queue.repeat,
        error: audio.error,
    })
}

/// Starts whatever the queue now points at (or stops), and tells the UI.
fn play_current(app: &AppHandle, item: Option<QueueItem>) {
    let state = app.state::<AppState>();
    match item {
        Some(item) => state.audio.load(item.path, true),
        None => state.audio.stop(),
    }
    // let the audio thread pick it up before reporting
    std::thread::sleep(Duration::from_millis(30));
    if let Ok(s) = status(&state) {
        let _ = app.emit("player", s);
    }
}

fn queue_items(state: &AppState, ids: &[i64]) -> CmdResult<Vec<QueueItem>> {
    let db = state.db.lock().map_err(err)?;
    ids.iter().map(|&id| db.track(id).map(|t| QueueItem { id, path: PathBuf::from(t.path) })).collect()
}

/// Files that open ReSound arrive before setup has opened the library: they wait here for it.
static WAITING: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Music opened from outside (ReFile, or Open With in Finder): the folder it's in becomes the queue,
/// starting at it (several files opened at once: just those).
fn open_files(app: &AppHandle, opened: Vec<PathBuf>) {
    if app.try_state::<AppState>().is_none() {
        if let Ok(mut waiting) = WAITING.lock() {
            waiting.extend(opened);
        }
        return;
    }
    let opened: Vec<PathBuf> = opened.into_iter().filter(|p| scan::is_audio(p)).collect();
    let Some(first) = opened.first().cloned() else { return };
    let mut paths = if opened.len() > 1 {
        opened
    } else {
        first
            .parent()
            .and_then(|dir| std::fs::read_dir(dir).ok())
            .map(|dir| {
                dir.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file() && scan::is_audio(p))
                    .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
                    .collect()
            })
            .unwrap_or_else(|| vec![first.clone()])
    };
    paths.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    let start = paths.iter().position(|p| *p == first).unwrap_or(0);

    let state = app.state::<AppState>();
    let ids = match state.db.lock().map_err(err).and_then(|db| scan::add(&db, &paths)) {
        Ok(ids) => ids,
        Err(e) => return eprintln!("ReSound: couldn't open {}: {e}", first.display()),
    };
    let Ok(items) = queue_items(&state, &ids) else { return };
    let item = state.queue.lock().ok().and_then(|mut q| {
        q.set(items, start);
        q.current().cloned()
    });
    play_current(app, item);
    // the library may have new tracks
    let _ = app.emit("scan", serde_json::json!({ "finished": true }));
}

// ── library ──

#[tauri::command]
fn music_folder(state: State<AppState>) -> CmdResult<Option<String>> {
    state.db.lock().map_err(err)?.setting("folder")
}

#[tauri::command]
fn set_music_folder(app: AppHandle, state: State<AppState>, folder: String) -> CmdResult<()> {
    state.db.lock().map_err(err)?.set_setting("folder", &folder)?;
    rescan(app, state)
}

/// Scans the music folder in the background; progress and the end come as `scan` events.
#[tauri::command]
fn rescan(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let Some(folder) = state.db.lock().map_err(err)?.setting("folder")? else {
        return Err("Choose a music folder first.".into());
    };
    {
        let mut busy = state.scanning.lock().map_err(err)?;
        if *busy {
            return Ok(());
        }
        *busy = true;
    }
    let db_path = state.db_path.clone();
    std::thread::spawn(move || {
        // a connection of its own, so the library stays usable while scanning
        let result = Db::open(&db_path).and_then(|db| {
            scan::scan(&db, &PathBuf::from(&folder), |done, total| {
                let _ = app.emit("scan", serde_json::json!({ "done": done, "total": total, "finished": false }));
            })
        });
        if let Ok(mut busy) = app.state::<AppState>().scanning.lock() {
            *busy = false;
        }
        let payload = match result {
            Ok(r) => serde_json::json!({ "finished": true, "result": r }),
            Err(e) => serde_json::json!({ "finished": true, "error": e }),
        };
        let _ = app.emit("scan", payload);
    });
    Ok(())
}

#[tauri::command]
fn tracks(state: State<AppState>) -> CmdResult<Vec<Track>> {
    state.db.lock().map_err(err)?.tracks()
}

// ── playlists ──

#[tauri::command]
fn playlists(state: State<AppState>) -> CmdResult<Vec<Playlist>> {
    state.db.lock().map_err(err)?.playlists()
}

#[tauri::command]
fn playlist_tracks(state: State<AppState>, id: i64) -> CmdResult<Vec<Track>> {
    state.db.lock().map_err(err)?.playlist_tracks(id)
}

#[tauri::command]
fn create_playlist(state: State<AppState>, name: String, track_ids: Vec<i64>) -> CmdResult<i64> {
    let db = state.db.lock().map_err(err)?;
    let id = db.create_playlist(&name, now())?;
    db.add_to_playlist(id, &track_ids)?;
    Ok(id)
}

#[tauri::command]
fn rename_playlist(state: State<AppState>, id: i64, name: String) -> CmdResult<()> {
    state.db.lock().map_err(err)?.rename_playlist(id, &name)
}

#[tauri::command]
fn delete_playlist(state: State<AppState>, id: i64) -> CmdResult<()> {
    state.db.lock().map_err(err)?.delete_playlist(id)
}

#[tauri::command]
fn add_to_playlist(state: State<AppState>, id: i64, track_ids: Vec<i64>) -> CmdResult<()> {
    state.db.lock().map_err(err)?.add_to_playlist(id, &track_ids)
}

#[tauri::command]
fn remove_from_playlist(state: State<AppState>, id: i64, index: usize) -> CmdResult<()> {
    state.db.lock().map_err(err)?.remove_from_playlist(id, index)
}

// ── playback ──

/// Plays `track_ids` as the new queue, starting at `start`.
#[tauri::command]
fn play(app: AppHandle, state: State<AppState>, track_ids: Vec<i64>, start: usize) -> CmdResult<()> {
    let items = queue_items(&state, &track_ids)?;
    let item = {
        let mut q = state.queue.lock().map_err(err)?;
        q.set(items, start);
        q.current().cloned()
    };
    play_current(&app, item);
    Ok(())
}

#[tauri::command]
fn toggle_play(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let audio = state.audio.status.lock().map_err(err)?.clone();
    if audio.playing {
        state.audio.pause();
    } else if audio.position_ms == 0 && !state.queue.lock().map_err(err)?.is_empty() {
        // stopped at the end (or never started): start the current track again
        let item = state.queue.lock().map_err(err)?.current().cloned();
        play_current(&app, item);
        return Ok(());
    } else {
        state.audio.play();
    }
    Ok(())
}

#[tauri::command]
fn next(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let item = state.queue.lock().map_err(err)?.advance(false).cloned();
    if item.is_some() {
        play_current(&app, item);
    }
    Ok(())
}

/// Like Music: more than 3 s in → back to the start of the track, otherwise the previous one.
#[tauri::command]
fn previous(app: AppHandle, state: State<AppState>) -> CmdResult<()> {
    let position = state.audio.status.lock().map_err(err)?.position_ms;
    if position > 3000 {
        state.audio.seek(Duration::ZERO);
        return Ok(());
    }
    let item = state.queue.lock().map_err(err)?.back().cloned();
    play_current(&app, item);
    Ok(())
}

#[tauri::command]
fn seek(state: State<AppState>, position_ms: u64) -> CmdResult<()> {
    state.audio.seek(Duration::from_millis(position_ms));
    Ok(())
}

#[tauri::command]
fn set_shuffle(state: State<AppState>, on: bool) -> CmdResult<()> {
    state.queue.lock().map_err(err)?.set_shuffle(on);
    Ok(())
}

#[tauri::command]
fn cycle_repeat(state: State<AppState>) -> CmdResult<Repeat> {
    Ok(state.queue.lock().map_err(err)?.cycle_repeat())
}

#[tauri::command]
fn play_next(state: State<AppState>, track_ids: Vec<i64>) -> CmdResult<()> {
    let items = queue_items(&state, &track_ids)?;
    state.queue.lock().map_err(err)?.play_next(items);
    Ok(())
}

#[tauri::command]
fn add_to_queue(state: State<AppState>, track_ids: Vec<i64>) -> CmdResult<()> {
    let items = queue_items(&state, &track_ids)?;
    state.queue.lock().map_err(err)?.append(items);
    Ok(())
}

#[tauri::command]
fn player_status(state: State<AppState>) -> CmdResult<Status> {
    status(&state)
}

/// The tracks still to come, in play order.
#[tauri::command]
fn upcoming(state: State<AppState>) -> CmdResult<Vec<Track>> {
    let ids = state.queue.lock().map_err(err)?.upcoming();
    let db = state.db.lock().map_err(err)?;
    Ok(ids.into_iter().filter_map(|id| db.track(id).ok()).collect())
}

#[tauri::command]
fn reveal_in_finder(path: String) -> CmdResult<()> {
    std::process::Command::new("open").args(["-R", &path]).status().map(|_| ()).map_err(err)
}

/// No title bar at all (decorations are off in tauri.conf.json): macOS rims every titled window with
/// a light line that can't be turned off. The window is clear instead, its page is rounded here like a
/// window's corners, and the UI draws its own close / minimise / zoom buttons. No shadow either
/// (tauri.conf.json).
#[cfg(target_os = "macos")]
fn round_window(window: &tauri::WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let _ = window.with_webview(|platform| {
        let view = platform.inner() as *mut AnyObject;
        if view.is_null() {
            return;
        }
        // SAFETY: `inner()` is the live WKWebView (an NSView); this runs on the main thread.
        unsafe {
            let _: () = msg_send![view, setWantsLayer: true];
            let layer: *mut AnyObject = msg_send![view, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: 12.0f64];
                let _: () = msg_send![layer, setMasksToBounds: true];
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("library.sqlite");
            let db = Db::open(&db_path)?;
            // first start: use ~/Music
            if db.setting("folder")?.is_none() {
                if let Some(music) = dirs::audio_dir() {
                    db.set_setting("folder", &music.to_string_lossy())?;
                }
            }

            // when a track ends by itself, the queue moves on from here
            let handle = app.handle().clone();
            let audio = Audio::spawn(move || {
                let state = handle.state::<AppState>();
                let item = state.queue.lock().ok().and_then(|mut q| q.advance(true).cloned());
                play_current(&handle, item);
            });

            app.manage(AppState {
                db: Mutex::new(db),
                db_path,
                queue: Mutex::new(Queue::default()),
                audio,
                scanning: Mutex::new(false),
            });

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                round_window(&window);
            }
            let waiting = WAITING.lock().map(|mut w| std::mem::take(&mut *w)).unwrap_or_default();
            if !waiting.is_empty() {
                open_files(app.handle(), waiting);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            music_folder,
            set_music_folder,
            rescan,
            tracks,
            playlists,
            playlist_tracks,
            create_playlist,
            rename_playlist,
            delete_playlist,
            add_to_playlist,
            remove_from_playlist,
            play,
            toggle_play,
            next,
            previous,
            seek,
            set_shuffle,
            cycle_repeat,
            play_next,
            add_to_queue,
            player_status,
            upcoming,
            reveal_in_finder,
        ])
        .build(tauri::generate_context!())
        .expect("error while running ReSound")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = event {
                open_files(app, urls.iter().filter_map(|u| u.to_file_path().ok()).collect());
            }
        });
}
