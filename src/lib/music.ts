// Bridge to Rust (src-tauri/src/lib.rs) and helpers to group the library.
import { invoke } from '@tauri-apps/api/core';

export interface Track {
	id: number;
	path: string;
	title: string;
	artist: string;
	album: string;
	albumArtist: string;
	trackNo: number | null;
	discNo: number | null;
	year: number | null;
	durationMs: number;
}

export interface Playlist {
	id: number;
	name: string;
	trackCount: number;
}

export type Repeat = 'off' | 'all' | 'one';

export interface Status {
	track: Track | null;
	playing: boolean;
	positionMs: number;
	shuffle: boolean;
	repeat: Repeat;
	error: string | null;
}

export const api = {
	musicFolder: () => invoke<string | null>('music_folder'),
	setMusicFolder: (folder: string) => invoke<void>('set_music_folder', { folder }),
	rescan: () => invoke<void>('rescan'),
	tracks: () => invoke<Track[]>('tracks'),

	playlists: () => invoke<Playlist[]>('playlists'),
	playlistTracks: (id: number) => invoke<Track[]>('playlist_tracks', { id }),
	createPlaylist: (name: string, trackIds: number[] = []) => invoke<number>('create_playlist', { name, trackIds }),
	renamePlaylist: (id: number, name: string) => invoke<void>('rename_playlist', { id, name }),
	deletePlaylist: (id: number) => invoke<void>('delete_playlist', { id }),
	addToPlaylist: (id: number, trackIds: number[]) => invoke<void>('add_to_playlist', { id, trackIds }),
	removeFromPlaylist: (id: number, index: number) => invoke<void>('remove_from_playlist', { id, index }),

	play: (trackIds: number[], start = 0) => invoke<void>('play', { trackIds, start }),
	togglePlay: () => invoke<void>('toggle_play'),
	next: () => invoke<void>('next'),
	previous: () => invoke<void>('previous'),
	seek: (positionMs: number) => invoke<void>('seek', { positionMs: Math.max(0, Math.round(positionMs)) }),
	setShuffle: (on: boolean) => invoke<void>('set_shuffle', { on }),
	cycleRepeat: () => invoke<Repeat>('cycle_repeat'),
	playNext: (trackIds: number[]) => invoke<void>('play_next', { trackIds }),
	addToQueue: (trackIds: number[]) => invoke<void>('add_to_queue', { trackIds }),
	status: () => invoke<Status>('player_status'),
	upcoming: () => invoke<Track[]>('upcoming'),
	reveal: (path: string) => invoke<void>('reveal_in_finder', { path }),
};

/** 3:07, 1:02:45 */
export function formatTime(ms: number): string {
	const total = Math.max(0, Math.floor(ms / 1000));
	const h = Math.floor(total / 3600);
	const m = Math.floor((total % 3600) / 60);
	const s = String(total % 60).padStart(2, '0');
	return h ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

export interface Group {
	key: string;
	title: string;
	subtitle: string;
	tracks: Track[];
}

const unknown = (s: string, fallback: string) => s || fallback;
const byName = (a: Group, b: Group) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base', numeric: true });

/** Albums: one per album artist + album name; tracks keep album order from Rust. */
export function albums(tracks: Track[]): Group[] {
	const map = new Map<string, Group>();
	for (const t of tracks) {
		const key = `${t.albumArtist}\u0000${t.album}`;
		let g = map.get(key);
		if (!g) {
			g = { key, title: unknown(t.album, 'Unknown album'), subtitle: unknown(t.albumArtist, 'Unknown artist'), tracks: [] };
			map.set(key, g);
		}
		g.tracks.push(t);
	}
	return [...map.values()].sort(byName);
}

/** Artists by album artist, their albums in order. */
export function artists(tracks: Track[]): Group[] {
	const map = new Map<string, Group>();
	for (const t of tracks) {
		const name = unknown(t.albumArtist || t.artist, 'Unknown artist');
		let g = map.get(name);
		if (!g) {
			g = { key: name, title: name, subtitle: '', tracks: [] };
			map.set(name, g);
		}
		g.tracks.push(t);
	}
	for (const g of map.values()) {
		const albumCount = new Set(g.tracks.map(t => t.album)).size;
		g.subtitle = `${albumCount} ${albumCount === 1 ? 'album' : 'albums'}`;
	}
	return [...map.values()].sort(byName);
}

export function songs(tracks: Track[]): Track[] {
	return [...tracks].sort((a, b) => a.title.localeCompare(b.title, undefined, { sensitivity: 'base', numeric: true }));
}

export function matches(t: Track, q: string): boolean {
	return [t.title, t.artist, t.album, t.albumArtist].some(s => s.toLowerCase().includes(q));
}
