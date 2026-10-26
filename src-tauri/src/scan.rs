//! Finds audio files in the music folder and reads their tags.
//! Rescans only re-read files whose modification time changed.

use crate::db::{Db, TrackInfo};
use lofty::prelude::*;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub const AUDIO_EXTS: [&str; 10] = ["mp3", "m4a", "aac", "alac", "flac", "wav", "aif", "aiff", "ogg", "opus"];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

/// "03 - Song" / "03. Song" / "03 Song" → (Some(3), "Song")
fn split_number(stem: &str) -> (Option<u32>, String) {
    let digits: String = stem.chars().take_while(char::is_ascii_digit).collect();
    let rest = stem[digits.len()..].trim_start_matches([' ', '.', '-', '_']).trim();
    if digits.is_empty() || digits.len() > 3 || rest.is_empty() || rest.len() == stem.len() - digits.len() {
        return (None, stem.to_owned());
    }
    (digits.parse().ok(), rest.to_owned())
}

/// Tags of one file. Without tags, a usual Artist / Album / 01 Song.mp3 layout is read
/// from the folder and file names.
pub fn read_info(path: &Path) -> TrackInfo {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name_of = |p: Option<&Path>| p.and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let album = name_of(path.parent());
    let artist = name_of(path.parent().and_then(|p| p.parent()));
    let (track_no, title) = split_number(&stem);
    let mut info = TrackInfo {
        title,
        album,
        artist: artist.clone(),
        album_artist: artist,
        track_no,
        ..Default::default()
    };

    let Ok(file) = lofty::read_from_path(path) else { return info };
    info.duration_ms = file.properties().duration().as_millis() as i64;
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        let text = |v: Option<std::borrow::Cow<'_, str>>| v.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
        if let Some(t) = text(tag.title()) {
            info.title = t;
        }
        if let Some(a) = text(tag.album()) {
            info.album = a;
        }
        if let Some(a) = text(tag.artist()) {
            info.artist = a;
        }
        info.album_artist = tag
            .get_string(ItemKey::AlbumArtist)
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| info.artist.clone());
        info.track_no = tag.track().or(info.track_no);
        info.disc_no = tag.disk();
        info.year = tag.date().map(|d| d.year as u32);
    }
    info
}

#[derive(Debug, Default, serde::Serialize)]
pub struct ScanResult {
    pub added_or_updated: usize,
    pub removed: usize,
    pub total: usize,
}

/// Brings the library in line with `folder`. `progress(done, total)` is called as files are read.
pub fn scan(db: &Db, folder: &Path, mut progress: impl FnMut(usize, usize)) -> Result<ScanResult, String> {
    let files: Vec<_> = WalkDir::new(folder)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && !e.file_name().to_string_lossy().starts_with('.') && is_audio(e.path()))
        .map(|e| e.into_path())
        .collect();

    let mut known = db.known_files()?;
    let mut result = ScanResult { total: files.len(), ..Default::default() };
    db.begin()?;
    for (i, path) in files.iter().enumerate() {
        let key = path.to_string_lossy().into_owned();
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if known.remove(&key) != Some(mtime) {
            db.upsert_track(&key, mtime, &read_info(path))?;
            result.added_or_updated += 1;
        }
        if i % 25 == 0 {
            progress(i, files.len());
        }
    }
    // whatever is left in `known` is no longer in the folder
    let gone: Vec<String> = known.into_keys().collect();
    result.removed = gone.len();
    db.remove_tracks(&gone)?;
    db.commit()?;
    progress(files.len(), files.len());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_numbers_from_file_names() {
        assert_eq!(split_number("03 - Song"), (Some(3), "Song".into()));
        assert_eq!(split_number("12. Two Words"), (Some(12), "Two Words".into()));
        assert_eq!(split_number("1999"), (None, "1999".into()), "a year-like title stays");
        assert_eq!(split_number("2046 Theme"), (None, "2046 Theme".into()), "4 digits is not a track number");
        assert_eq!(split_number("Song"), (None, "Song".into()));
    }

    #[test]
    fn scan_finds_audio_falls_back_to_names_and_drops_missing() {
        let dir = std::env::temp_dir().join(format!("resound-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Some Album")).unwrap();
        // not real audio: tags can't be read, so names are used
        std::fs::write(dir.join("Some Album/01 Song.mp3"), b"x").unwrap();
        std::fs::write(dir.join("Some Album/notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("Some Album/.hidden.mp3"), b"x").unwrap();

        let db = Db::open_in_memory().unwrap();
        let first = scan(&db, &dir, |_, _| {}).unwrap();
        assert_eq!((first.total, first.added_or_updated), (1, 1));
        let t = &db.tracks().unwrap()[0];
        assert_eq!((t.title.as_str(), t.album.as_str(), t.track_no), ("Song", "Some Album", Some(1)));
        assert_eq!(t.artist, dir.file_name().unwrap().to_string_lossy(), "artist = the folder above the album");

        let again = scan(&db, &dir, |_, _| {}).unwrap();
        assert_eq!(again.added_or_updated, 0, "unchanged files are skipped");

        std::fs::remove_file(dir.join("Some Album/01 Song.mp3")).unwrap();
        let after = scan(&db, &dir, |_, _| {}).unwrap();
        assert_eq!(after.removed, 1);
        assert!(db.tracks().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
