//! Library storage: tracks found in the music folder, playlists and settings, in SQLite.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: i64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: i64,
}

/// What the scanner read from one file.
#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub duration_ms: i64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub track_count: i64,
}

pub struct Db {
    conn: Connection,
}

const TRACK_COLUMNS: &str =
    "t.id, t.path, t.title, t.artist, t.album, t.album_artist, t.track_no, t.disc_no, t.year, t.duration_ms";

fn track_from_row(r: &Row) -> rusqlite::Result<Track> {
    Ok(Track {
        id: r.get(0)?,
        path: r.get(1)?,
        title: r.get(2)?,
        artist: r.get(3)?,
        album: r.get(4)?,
        album_artist: r.get(5)?,
        track_no: r.get(6)?,
        disc_no: r.get(7)?,
        year: r.get(8)?,
        duration_ms: r.get(9)?,
    })
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path).map_err(err)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory().map_err(err)?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL").map_err(err)?;
        conn.pragma_update(None, "foreign_keys", "ON").map_err(err)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracks (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                path         TEXT NOT NULL UNIQUE,
                mtime        INTEGER NOT NULL,     -- to skip unchanged files on rescan
                title        TEXT NOT NULL,
                artist       TEXT NOT NULL,
                album        TEXT NOT NULL,
                album_artist TEXT NOT NULL,
                track_no     INTEGER,
                disc_no      INTEGER,
                year         INTEGER,
                duration_ms  INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlists (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS playlist_tracks (
                playlist_id INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
                track_id    INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
                position    INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )
        .map_err(err)?;
        Ok(Self { conn })
    }

    // ── settings ──

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .map_err(err)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute("INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = ?2", params![key, value])
            .map(|_| ())
            .map_err(err)
    }

    // ── tracks ──

    /// Album order: album artist, album, disc, track number, title.
    pub fn tracks(&self) -> Result<Vec<Track>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks t
                 ORDER BY t.album_artist COLLATE NOCASE, t.album COLLATE NOCASE,
                          COALESCE(t.disc_no, 1), COALESCE(t.track_no, 9999), t.title COLLATE NOCASE"
            ))
            .map_err(err)?;
        let rows = stmt.query_map([], track_from_row).map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    pub fn track(&self, id: i64) -> Result<Track> {
        self.conn
            .query_row(&format!("SELECT {TRACK_COLUMNS} FROM tracks t WHERE t.id = ?1"), params![id], track_from_row)
            .map_err(err)
    }

    /// path → mtime of everything known, for incremental rescans.
    pub fn known_files(&self) -> Result<std::collections::HashMap<String, i64>> {
        let mut stmt = self.conn.prepare("SELECT path, mtime FROM tracks").map_err(err)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    /// Inserts or refreshes a track, keeping its id (so playlists stay intact).
    pub fn upsert_track(&self, path: &str, mtime: i64, info: &TrackInfo) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO tracks (path, mtime, title, artist, album, album_artist, track_no, disc_no, year, duration_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(path) DO UPDATE SET
                    mtime = ?2, title = ?3, artist = ?4, album = ?5, album_artist = ?6,
                    track_no = ?7, disc_no = ?8, year = ?9, duration_ms = ?10",
                params![
                    path, mtime, info.title, info.artist, info.album, info.album_artist,
                    info.track_no, info.disc_no, info.year, info.duration_ms
                ],
            )
            .map(|_| ())
            .map_err(err)
    }

    pub fn remove_tracks(&self, paths: &[String]) -> Result<()> {
        for p in paths {
            self.conn.execute("DELETE FROM tracks WHERE path = ?1", params![p]).map_err(err)?;
        }
        Ok(())
    }

    pub fn begin(&self) -> Result<()> {
        self.conn.execute_batch("BEGIN").map_err(err)
    }

    pub fn commit(&self) -> Result<()> {
        self.conn.execute_batch("COMMIT").map_err(err)
    }

    // ── playlists ──

    pub fn playlists(&self) -> Result<Vec<Playlist>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT p.id, p.name, COUNT(pt.track_id) FROM playlists p
                 LEFT JOIN playlist_tracks pt ON pt.playlist_id = p.id
                 GROUP BY p.id ORDER BY p.name COLLATE NOCASE",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)?, track_count: r.get(2)? }))
            .map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    fn clean_name(name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the playlist a name.".into());
        }
        Ok(name.chars().take(80).collect())
    }

    pub fn create_playlist(&self, name: &str, now: i64) -> Result<i64> {
        self.conn
            .execute("INSERT INTO playlists (name, created_at) VALUES (?1, ?2)", params![Self::clean_name(name)?, now])
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_playlist(&self, id: i64, name: &str) -> Result<()> {
        self.conn
            .execute("UPDATE playlists SET name = ?1 WHERE id = ?2", params![Self::clean_name(name)?, id])
            .map(|_| ())
            .map_err(err)
    }

    pub fn delete_playlist(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM playlists WHERE id = ?1", params![id]).map(|_| ()).map_err(err)
    }

    pub fn playlist_tracks(&self, id: i64) -> Result<Vec<Track>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {TRACK_COLUMNS} FROM playlist_tracks pt JOIN tracks t ON t.id = pt.track_id
                 WHERE pt.playlist_id = ?1 ORDER BY pt.position"
            ))
            .map_err(err)?;
        let rows = stmt.query_map(params![id], track_from_row).map_err(err)?;
        rows.collect::<rusqlite::Result<_>>().map_err(err)
    }

    /// Appends tracks to the end of a playlist (a track may appear more than once, like in Music).
    pub fn add_to_playlist(&self, id: i64, track_ids: &[i64]) -> Result<()> {
        let start: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_tracks WHERE playlist_id = ?1", params![id], |r| r.get(0))
            .map_err(err)?;
        for (i, t) in track_ids.iter().enumerate() {
            self.conn
                .execute(
                    "INSERT INTO playlist_tracks (playlist_id, track_id, position) VALUES (?1, ?2, ?3)",
                    params![id, t, start + i as i64],
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Removes the entry at `index` in the playlist's order.
    pub fn remove_from_playlist(&self, id: i64, index: usize) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM playlist_tracks WHERE rowid = (
                    SELECT rowid FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position LIMIT 1 OFFSET ?2)",
                params![id, index as i64],
            )
            .map(|_| ())
            .map_err(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(title: &str, album: &str, track: u32) -> TrackInfo {
        TrackInfo {
            title: title.into(),
            artist: "Artist".into(),
            album: album.into(),
            album_artist: "Artist".into(),
            track_no: Some(track),
            duration_ms: 1000,
            ..Default::default()
        }
    }

    #[test]
    fn tracks_keep_album_order_and_ids_on_rescan() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track("/m/b2.mp3", 1, &info("Two", "B", 2)).unwrap();
        db.upsert_track("/m/b1.mp3", 1, &info("One", "B", 1)).unwrap();
        db.upsert_track("/m/a1.mp3", 1, &info("Alpha", "A", 1)).unwrap();
        let titles: Vec<_> = db.tracks().unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(titles, vec!["Alpha", "One", "Two"]);

        let id = db.tracks().unwrap()[1].id;
        db.upsert_track("/m/b1.mp3", 2, &info("One (remaster)", "B", 1)).unwrap();
        assert_eq!(db.track(id).unwrap().title, "One (remaster)", "rescan updates in place");
        assert_eq!(db.known_files().unwrap()["/m/b1.mp3"], 2);
    }

    #[test]
    fn playlists_add_remove_and_follow_deleted_tracks() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_track("/m/1.mp3", 1, &info("One", "A", 1)).unwrap();
        db.upsert_track("/m/2.mp3", 1, &info("Two", "A", 2)).unwrap();
        let ids: Vec<_> = db.tracks().unwrap().into_iter().map(|t| t.id).collect();

        let p = db.create_playlist("  Evening ", 0).unwrap();
        assert!(db.create_playlist("   ", 0).is_err());
        db.add_to_playlist(p, &[ids[1], ids[0], ids[1]]).unwrap();
        let order: Vec<_> = db.playlist_tracks(p).unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(order, vec!["Two", "One", "Two"]);

        db.remove_from_playlist(p, 1).unwrap();
        let order: Vec<_> = db.playlist_tracks(p).unwrap().into_iter().map(|t| t.title).collect();
        assert_eq!(order, vec!["Two", "Two"]);

        db.remove_tracks(&["/m/2.mp3".into()]).unwrap();
        assert!(db.playlist_tracks(p).unwrap().is_empty(), "deleted files leave playlists too");
        let lists = db.playlists().unwrap();
        assert_eq!((lists[0].name.as_str(), lists[0].track_count), ("Evening", 0));

        db.rename_playlist(p, "Night").unwrap();
        db.delete_playlist(p).unwrap();
        assert!(db.playlists().unwrap().is_empty());
    }
}
