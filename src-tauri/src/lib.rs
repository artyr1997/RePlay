//! ReSound: a quiet music player in the ReNote style.
//! Rust scans the music folder, keeps the library and playlists in SQLite and
//! plays the audio; the Svelte UI talks to it through these commands, the
//! `player` / `scan` events and `resound://localhost/cover/<track id>`.

mod db;
mod player;
mod scan;

use db::{Db, Playlist, Track};
use player::{Audio, QueueItem, Queue, Repeat};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tauri::http::{Response, StatusCode};
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

// ── covers over resound:// ──

fn serve_cover(app: &AppHandle, path: &str) -> Response<Vec<u8>> {
    let id = path.trim_matches('/').strip_prefix("cover/").and_then(|s| s.parse::<i64>().ok());
    let file = id.and_then(|id| app.state::<AppState>().db.lock().ok()?.track(id).ok()).map(|t| PathBuf::from(t.path));
    let cover = file.as_deref().and_then(scan::cover);
    let builder = Response::builder().header("Access-Control-Allow-Origin", "*");
    match cover {
        Some((bytes, mime)) => builder
            .status(StatusCode::OK)
            .header("Content-Type", mime)
            .header("Cache-Control", "max-age=86400")
            .body(bytes),
        None => builder.status(StatusCode::NOT_FOUND).body(Vec::new()),
    }
    .unwrap_or_else(|_| Response::new(Vec::new()))
}

/// The title bar is transparent (see tauri.conf.json) and the UI draws its own
/// vertical close / minimise / zoom buttons, so the native ones are hidden.
#[cfg(target_os = "macos")]
fn hide_native_window_buttons(window: &tauri::WebviewWindow) {
    use objc2_app_kit::{NSWindow, NSWindowButton};

    let Ok(ptr) = window.ns_window() else { return };
    // SAFETY: Tauri returns the live NSWindow of this webview window; setup runs on the main thread.
    let ns_window = unsafe { &*(ptr as *const NSWindow) };
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_asynchronous_uri_scheme_protocol("resound", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_owned();
            std::thread::spawn(move || responder.respond(serve_cover(&app, &path)));
        })
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
                hide_native_window_buttons(&window);
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
        .run(tauri::generate_context!())
        .expect("error while running ReSound");
}
