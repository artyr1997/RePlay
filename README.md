# ReSound

A quiet music player for macOS in the style of ReNote. Rust (Tauri) scans your music
folder, keeps the library and playlists in SQLite and plays the audio (rodio); the
interface is Svelte.

- **Albums · Artists · Songs · Lists**: two columns (group → songs), or a song list
- **Radio** (LIVE at the top right, or 2; SONG at the top left, or 1, back; shuffle and repeat on S / R): stations from Radio Browser (radio-browser.info), genres on the left and their stations on the right (← → between them), / searches station names. Enter plays one; the player shows the song on air and LIVE; PREV / NEXT go along the list; a pause lets the station go and PLAY tunes in again. F (or a right click) keeps a station in Favorites. MP3, AAC and Ogg Vorbis streams.
- **Player**: PLAY / PAUSE, PREV / NEXT, shuffle (S), repeat (R: all / one), QUEUE (up next),
  click the line to seek. The next song starts from Rust, also with the window hidden.
- **Playlists**: create, rename, delete; add songs or whole albums; remove songs
- **Actions** (E or right click): play, play next, add to queue, add to playlist, show in Finder
- Files: MP3, AAC / M4A, ALAC, FLAC, WAV, AIFF, OGG. Without tags, an
  `Artist / Album / 01 Song.mp3` layout is read from the folder and file names.
- Keys: Space play/pause · the Mac's media keys ⏮ ⏯ ⏭ (F7 F8 F9, also the headphones' buttons and Control Centre) previous / play-pause / next · ↑ ↓ ← → move · Enter play ·
  1–4 tabs · / search · S shuffle · R repeat · Q queue · N new playlist · I info

## Run

```sh
npm install
npm run desktop        # dev window with live reload
npm run desktop:build  # ReSound.app + .dmg in src-tauri/target/release/bundle/
```

Rust tests: `cd src-tauri && cargo test`. The music folder defaults to `~/Music`
(change it in INFO). Library data: `~/Library/Application Support/com.artursavytskyi.resound/`.
