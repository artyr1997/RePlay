<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import WindowControls from './lib/WindowControls.svelte';
	import Popup from './lib/Popup.svelte';
	import Video from './lib/Video.svelte';
	import { invoke } from '@tauri-apps/api/core';
	import { getCurrentWebview } from '@tauri-apps/api/webview';
	import { scrollFade } from './lib/scrollFade';
	import { api, songs, matches, formatTime, type Track, type Status, type Station } from './lib/music';

	type PopupKind = 'none' | 'actions' | 'queue' | 'info';

	let allTracks: Track[] = $state([]);
	let ti = $state(0);
	let query = $state('');
	let loaded = $state(false);
	let folder: string | null = $state(null);
	let scan: { done: number; total: number } | null = $state(null);

	// ── mini player: a small window with just the song and the controls ──
	const MINI = { width: 440, height: 300 };
	const FULL = { width: 1440, height: 900 };
	let winWidth = $state(window.innerWidth);
	let winHeight = $state(window.innerHeight);
	// a small window shows the mini player, however it got small
	let mini = $derived(winWidth < 760 || winHeight < 560);

	async function setMini(on: boolean) {
		const size = on ? MINI : FULL;
		await getCurrentWindow().setSize(new LogicalSize(size.width, size.height));
	}

	let status: Status = $state({ track: null, station: null, onAir: null, playing: false, positionMs: 0, shuffle: false, repeat: 'off', error: null });

	let searchInput: HTMLInputElement | undefined = $state();
	let trackList: HTMLDivElement | undefined = $state();

	// ── what is shown: every song, A to Z ──

	let q = $derived(query.trim().toLowerCase());
	let tracks: Track[] = $derived(songs(q ? allTracks.filter(t => matches(t, q)) : allTracks));
	let track: Track | undefined = $derived(tracks[ti]);

	// ── loading ──

	async function loadLibrary() {
		[allTracks, folder] = await Promise.all([api.tracks(), api.musicFolder()]);
		loaded = true;
		ti = Math.min(ti, Math.max(tracks.length - 1, 0));
	}

	// ── playback ──

	async function playTracks(list: Track[], start = 0) {
		if (!list.length) return;
		await api.play(list.map(t => t.id), start);
		await refreshStatus();
	}

	async function refreshStatus() {
		status = await api.status();
	}

	// S / R (no buttons for them): what they turned to shows for a moment
	async function toggleShuffle() {
		await api.setShuffle(!status.shuffle);
		await refreshStatus();
		showToast(status.shuffle ? 'SHUF' : 'ORDER');
	}

	async function cycleRepeat() {
		const repeat = await api.cycleRepeat();
		await refreshStatus();
		showToast({ all: 'REPT', one: 'ONE', off: 'ONCE' }[repeat]);
	}

	function seekTo(e: MouseEvent) {
		const t = status.track;
		if (!t) return;
		const bar = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const fraction = Math.min(1, Math.max(0, (e.clientX - bar.left) / bar.width));
		api.seek(fraction * t.durationMs).then(refreshStatus);
	}

	let progress = $derived(status.track && status.track.durationMs ? Math.min(1, status.positionMs / status.track.durationMs) : 0);

	// ── radio: genres on the left, their stations on the right (Radio Browser's, through Rust) ──

	type Mode = 'songs' | 'radio';
	let mode = $state<Mode>('songs');
	const FAVORITES = 'Favorites';
	let genres: string[] = $state([]);
	let favorites: Station[] = $state([]);
	let gi = $state(0);
	let si = $state(0);
	/** which column the arrows move in */
	let side = $state<'genres' | 'stations'>('genres');
	let byGenre: Record<string, Station[]> = $state({});
	let found: Station[] = $state([]);
	let radioLoading = $state(false);
	let radioError = $state('');
	let genreList: HTMLDivElement | undefined = $state();
	let stationList: HTMLDivElement | undefined = $state();

	let genreNames = $derived(favorites.length ? [FAVORITES, ...genres] : genres);
	let genre = $derived(genreNames[gi] ?? '');
	let stations: Station[] = $derived(q ? found : genre === FAVORITES ? favorites : (byGenre[genre] ?? []));
	let station: Station | undefined = $derived(stations[si]);
	let kept = $derived(new Set(favorites.map(s => s.uuid)));

	async function setMode(next: Mode) {
		if (mode === next) return;
		mode = next;
		query = '';
		if (next === 'radio' && !genres.length) {
			[genres, favorites] = await Promise.all([api.radioGenres(), api.radioFavorites()]);
			gi = 0;
			await loadGenre();
		}
	}

	async function loadGenre() {
		si = 0;
		const name = genre;
		if (!name || name === FAVORITES || byGenre[name]) return;
		radioLoading = true;
		radioError = '';
		try {
			byGenre[name] = await api.radioStations(name);
		} catch (e) {
			radioError = String(e);
		} finally {
			radioLoading = false;
		}
	}

	// the search, in radio: station names, asked a moment after the typing stops
	let searchTimer: ReturnType<typeof setTimeout> | null = null;
	$effect(() => {
		const text = q;
		if (mode !== 'radio') return;
		if (searchTimer) clearTimeout(searchTimer);
		if (!text) return void (found = []);
		searchTimer = setTimeout(async () => {
			radioLoading = true;
			radioError = '';
			try {
				found = await api.radioSearch(text);
				si = 0;
				side = 'stations';
			} catch (e) {
				radioError = String(e);
			} finally {
				radioLoading = false;
			}
		}, 400);
	});

	async function playStation(index = si) {
		const s = stations[index];
		if (!s) return;
		si = index;
		await api.playStation(s, stations);
		await refreshStatus();
	}

	async function toggleFavorite(s: Station | undefined = station) {
		if (!s) return;
		const on = !kept.has(s.uuid);
		await api.setRadioFavorite(s, on);
		const wasFavorites = genre === FAVORITES;
		favorites = await api.radioFavorites();
		// the left column may have gained or lost Favorites: stay on the same genre
		if (!wasFavorites) gi = Math.max(0, genreNames.indexOf(genre));
		else if (!favorites.length) gi = 0;
		showToast(on ? 'KEPT' : 'LET GO');
	}

	async function moveRadio(delta: number) {
		if (side === 'genres' && !q) {
			if (!genreNames.length) return;
			gi = (gi + delta + genreNames.length) % genreNames.length;
			await loadGenre();
			await tick();
			genreList?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
		} else {
			if (!stations.length) return;
			si = (si + delta + stations.length) % stations.length;
			await tick();
			stationList?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
		}
	}

	function radioKeys(e: KeyboardEvent): boolean {
		if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			moveRadio(e.key === 'ArrowDown' ? 1 : -1);
		} else if (e.key === 'ArrowRight') {
			side = 'stations';
		} else if (e.key === 'ArrowLeft' && !q) {
			side = 'genres';
		} else if (e.key === 'Enter') {
			if (side === 'genres' && !q) side = 'stations';
			else playStation();
		} else if (e.code === 'KeyF') {
			toggleFavorite();
		} else {
			return false;
		}
		e.preventDefault();
		return true;
	}

	// ── selection ──

	async function move(delta: number) {
		if (!tracks.length) return;
		ti = (ti + delta + tracks.length) % tracks.length;
		await tick();
		trackList?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
	}

	// ── popups ──

	let popup = $state<PopupKind>('none');
	let target: { track: Track; index: number } | null = $state(null);
	let selectedAction = $state(0);
	let upcoming: Track[] = $state([]);
	let toast = $state('');
	let toastTimer: ReturnType<typeof setTimeout> | null = null;

	function showToast(text: string) {
		toast = text;
		if (toastTimer) clearTimeout(toastTimer);
		toastTimer = setTimeout(() => (toast = ''), 1400);
	}

	type Action = { label: string; run: () => unknown };

	let actions: Action[] = $derived.by(() => {
		const t = target;
		if (!t) return [];
		const add = async (fn: (ids: number[]) => Promise<void>, msg: string) => {
			await fn([t.track.id]);
			closePopup();
			showToast(msg);
		};
		return [
			{ label: 'Play', run: () => { closePopup(); playTracks(tracks, t.index); } },
			{ label: 'Play next', run: () => add(api.playNext, 'NEXT') },
			{ label: 'Add to queue', run: () => add(api.addToQueue, 'QUEUED') },
			{ label: 'Show in Finder', run: () => { closePopup(); api.reveal(t.track.path); } },
		];
	});

	function openActions(index = ti) {
		const t = tracks[index];
		if (!t) return;
		target = { track: t, index };
		selectedAction = 0;
		popup = 'actions';
	}

	async function openQueue() {
		upcoming = await api.upcoming();
		popup = 'queue';
	}

	function closePopup() {
		popup = 'none';
	}

	async function chooseFolder() {
		const picked = await openDialog({ directory: true, defaultPath: folder ?? undefined });
		if (typeof picked === 'string') {
			await api.setMusicFolder(picked);
			folder = picked;
			closePopup();
		}
	}

	// ── videos: opened from outside, dropped on the window or picked with O (src-tauri/src/lib.rs) ──
	let video: { paths: string[]; start: number } | null = $state(null);
	const takeVideo = () =>
		invoke<{ paths: string[]; start: number } | null>('take_video').then(async v => {
			if (!v) return;
			// the same again, or another: shown anew, from its start
			video = null;
			await tick();
			video = v;
		});

	async function pickVideo() {
		const picked = await openDialog({
			multiple: true,
			filters: [{ name: 'Video', extensions: ['mp4', 'm4v', 'mov', 'webm'] }],
		});
		const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : [];
		if (paths.length) invoke('open_paths', { paths });
	}

	// ── keyboard ──

	function handleKeydown(e: KeyboardEvent) {
		const mod = e.metaKey || e.ctrlKey;

		// a video has its own keys (lib/Video.svelte)
		if (video) return;
		if (e.code === 'KeyO' && !e.altKey && !e.shiftKey && (mod || (e.target as HTMLElement)?.tagName !== 'INPUT')) {
			e.preventDefault();
			pickVideo();
			return;
		}

		if (mini) {
			if (e.key === ' ') {
				e.preventDefault();
				api.togglePlay().then(refreshStatus);
			} else if (!mod && e.code === 'KeyM') {
				setMini(false);
			}
			return;
		}

		if (popup !== 'none') {
			if (e.key === 'Escape') {
				e.preventDefault();
				closePopup();
				return;
			}
			if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && popup === 'actions') {
				e.preventDefault();
				const d = e.key === 'ArrowDown' ? 1 : -1;
				selectedAction = (selectedAction + d + actions.length) % actions.length;
			} else if (e.key === 'Enter') {
				e.preventDefault();
				if (popup === 'actions') actions[selectedAction]?.run();
				else closePopup();
			}
			return;
		}

		if (e.target === searchInput) {
			if (mode === 'radio' && (e.key === 'ArrowDown' || e.key === 'ArrowUp' || e.key === 'Enter')) {
				e.preventDefault();
				side = 'stations';
				if (e.key === 'Enter') playStation();
				else moveRadio(e.key === 'ArrowDown' ? 1 : -1);
				return;
			}
			if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
				e.preventDefault();
				move(e.key === 'ArrowDown' ? 1 : -1);
			} else if (e.key === 'Enter') {
				e.preventDefault();
				playTracks(tracks, ti);
			} else if (e.key === 'Escape') {
				query = '';
				searchInput?.blur();
			}
			return;
		}
		if ((e.target as HTMLElement)?.tagName === 'INPUT') return;

		if (!mod && (e.key === '1' || e.key === '2')) {
			setMode(e.key === '1' ? 'songs' : 'radio');
			return;
		}
		if (mode === 'radio' && !mod && radioKeys(e)) return;

		// previous / next: the Mac's media keys (⏮ ⏭, src-tauri/src/media.rs), not the arrows
		if (e.key === ' ') {
			e.preventDefault();
			api.togglePlay().then(refreshStatus);
		} else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
			e.preventDefault();
			move(e.key === 'ArrowDown' ? 1 : -1);
		} else if (e.key === 'Enter') {
			e.preventDefault();
			playTracks(tracks, ti);
		} else if (e.code === 'Slash') {
			e.preventDefault();
			searchInput?.focus();
		} else if (!mod && e.code === 'KeyS') {
			toggleShuffle();
		} else if (!mod && e.code === 'KeyR') {
			cycleRepeat();
		} else if (!mod && e.code === 'KeyQ') {
			openQueue();
		} else if (!mod && e.code === 'KeyE') {
			openActions();
		} else if (!mod && e.code === 'KeyI') {
			popup = 'info';
		} else if (!mod && e.code === 'KeyM') {
			setMini(true);
		}
	}

	onMount(() => {
		loadLibrary().then(() => api.rescan().catch(() => {}));
		refreshStatus();
		const timer = setInterval(refreshStatus, 400);
		takeVideo();
		const unlisten = [
			// to this window only (lib.rs: the one in front shows it)
			getCurrentWebview().listen('video', takeVideo),
			// files dropped on the window: videos shown, music played
			getCurrentWebview().onDragDropEvent(e => {
				if (e.payload.type === 'drop' && e.payload.paths.length) invoke('open_paths', { paths: e.payload.paths });
			}),
			listen<Status>('player', e => (status = e.payload)),
			listen<{ done?: number; total?: number; finished: boolean }>('scan', e => {
				if (e.payload.finished) {
					scan = null;
					loadLibrary();
				} else {
					scan = { done: e.payload.done ?? 0, total: e.payload.total ?? 0 };
				}
			}),
		];
		return () => {
			clearInterval(timer);
			unlisten.forEach(p => p.then(fn => fn()));
		};
	});

	const shortcuts = [
		['⌘ N', 'New window'],
		['Space', 'Play / pause'],
		['⏮ / ⏭', 'Previous / next song (the Mac\'s media keys)'],
		['↑ ↓', 'Move around'],
		['Enter', 'Play'],
		['/', 'Search'],
		['1 / 2', 'Songs / live radio'],
		['← →', 'Genres / stations (radio)'],
		['F', 'Keep a station'],
		['S / R', 'Shuffle / repeat'],
		['Q', 'Up next'],
		['E / right click', 'Actions'],
		['M', 'Mini player'],
		['O', 'Open a video (or drop one on the window)'],
		['I', 'Info'],
	];
</script>


<svelte:window
	bind:innerWidth={winWidth}
	bind:innerHeight={winHeight}
	onkeydown={handleKeydown}
	oncontextmenu={e => {
		if ((e.target as HTMLElement)?.tagName !== 'INPUT') e.preventDefault();
	}}
/>

<WindowControls />

{#if mini}
	<!-- mini player: song in the middle, the artist below, the controls under it -->
	<main class="mini fade-in">
		<button class="mini-expand" onclick={() => setMini(false)} title="Library (M)">LIBRARY</button>
		{#if status.error}
			<p class="mini-title error">{status.error}</p>
		{:else if status.station}
			<p class="mini-title">{status.onAir ?? status.station.name}</p>
			<p class="mini-artist">{status.onAir ? status.station.name : status.station.country}</p>
		{:else if status.track}
			<p class="mini-title">{status.track.title}</p>
			<p class="mini-artist">{status.track.artist || status.track.albumArtist}</p>
		{:else}
			<p class="mini-title idle">Nothing playing.</p>
		{/if}
		{#if !status.station}
			<div class="progress-row mini-progress">
				<span class="clock">{formatTime(status.positionMs)}</span>
				<button class="bar" onclick={seekTo} aria-label="Seek" disabled={!status.track}>
					<span class="bar-line"></span>
					<span class="bar-fill" style="width: {progress * 100}%"></span>
				</button>
				<span class="clock">{formatTime(status.track?.durationMs ?? 0)}</span>
			</div>
		{/if}
		<div class="mini-controls">
			<button class="mini-btn arrow" onclick={() => api.previous().then(refreshStatus)} title="Previous (⏮)" aria-label="Previous">←</button>
			<button class="play" onclick={() => api.togglePlay().then(refreshStatus)} title="Play / pause (Space)">
				<span class="ink-mark">{status.playing ? 'PAUSE' : 'PLAY'}</span>
			</button>
			<button class="mini-btn arrow" onclick={() => api.next().then(refreshStatus)} title="Next (⏭)" aria-label="Next">→</button>
		</div>
	</main>
{:else}

<!-- songs or the radio: the one shown inked with a vertical stroke (shuffle and repeat are on S / R) -->
<button class="side switch shuf" class:ink-mark-v={mode === 'songs'} onclick={() => setMode('songs')} title="Songs (1)">SONG</button>
<button class="side switch rept" class:ink-mark-v={mode === 'radio'} onclick={() => setMode('radio')} title="Radio (2)">LIVE</button>

<button class="side boxed nav-prev" onclick={() => api.previous().then(refreshStatus)} title="Previous (⏮)">PREV</button>
<button class="side boxed nav-next" onclick={() => api.next().then(refreshStatus)} title="Next (⏭)">NEXT</button>

<button class="side plain nav-info" onclick={() => (popup = 'info')} title="Info (I)">INFO</button>
<button class="side plain nav-mini" onclick={() => setMini(true)} title="Mini player (M)">MINI</button>

<!-- the search is the title -->
<input bind:this={searchInput} bind:value={query} class="search" type="text" placeholder={mode === 'radio' ? 'Search stations' : 'Search'} oninput={() => (ti = 0)} />

<main class="fade-in">
	{#if mode === 'radio'}
		<div class="radio">
			<div class="genres scroll" class:dim={!!q} bind:this={genreList} use:scrollFade>
				{#each genreNames as name, i (name)}
					<button
						class="row genre-row"
						class:active={i === gi}
						onclick={() => { gi = i; side = 'genres'; query = ''; loadGenre(); }}
					>
						<span class="name" class:ink-mark={i === gi && side === 'genres' && !q} class:chosen={i === gi}>{name}</span>
					</button>
				{/each}
			</div>
			<div class="stations scroll" bind:this={stationList} use:scrollFade>
				{#each stations as s, i (s.uuid)}
					<button
						class="row station-row"
						class:active={i === si}
						onclick={() => { si = i; side = 'stations'; }}
						ondblclick={() => playStation(i)}
						oncontextmenu={() => { si = i; toggleFavorite(s); }}
					>
						<span class="cell-title">
							{#if status.station?.uuid === s.uuid}<span class="dot"></span>{/if}
							<span class="name" class:ink-mark={i === si && side === 'stations'}>{s.name}</span>
							{#if kept.has(s.uuid) && genre !== FAVORITES}<span class="kept" title="Kept (F)">◆</span>{/if}
						</span>
						<span class="cell">{s.country}</span>
						<span class="cell kind">{s.codec} {s.bitrate || ''}</span>
					</button>
				{:else}
					<p class="empty">{radioLoading ? 'Tuning in…' : radioError || (q ? 'Nothing found.' : genre === FAVORITES ? 'No stations kept yet.' : 'No stations.')}</p>
				{/each}
			</div>
		</div>
	{:else if loaded && allTracks.length === 0 && !scan}
		<div class="welcome">
			<p>No music in <span class="ink-mark">{folder ? folder.split('/').pop() : 'your folder'}</span> yet.</p>
			<button class="pop-btn" onclick={chooseFolder}>CHOOSE FOLDER</button>
		</div>
	{:else}
		<div class="songs scroll" bind:this={trackList} use:scrollFade>
			{#each tracks as t, i (t.id)}
				<button
					class="row song-row"
					class:active={i === ti}
					onclick={() => (ti = i)}
					ondblclick={() => playTracks(tracks, i)}
					oncontextmenu={() => { ti = i; openActions(i); }}
				>
					<span class="cell-title">
						{#if status.track?.id === t.id}<span class="dot"></span>{/if}
						<span class="name" class:ink-mark={i === ti}>{t.title}</span>
					</span>
					<span class="cell">{t.artist || t.albumArtist}</span>
					<span class="cell time">{formatTime(t.durationMs)}</span>
				</button>
			{:else}
				<p class="empty">{q ? 'Nothing found.' : 'No songs yet.'}</p>
			{/each}
		</div>
	{/if}
</main>

<!-- player -->
<footer class="player">
	{#if status.error}
		<p class="now error">{status.error}</p>
	{:else if status.station}
		<!-- the song on air when the station names it, and the station -->
		<p class="now">
			{#if status.onAir}<span class="now-title">{status.onAir}</span><span class="now-artist"> — {status.station.name}</span>
			{:else}<span class="now-title">{status.station.name}</span>{/if}
		</p>
	{:else if status.track}
		<p class="now"><span class="now-title">{status.track.title}</span><span class="now-artist"> — {status.track.artist || status.track.albumArtist}</span></p>
	{:else}
		<p class="now idle">Nothing playing.</p>
	{/if}
	{#if !status.station}
		<div class="progress-row">
			<span class="clock">{formatTime(status.positionMs)}</span>
			<button class="bar" onclick={seekTo} aria-label="Seek" disabled={!status.track}>
				<span class="bar-line"></span>
				<span class="bar-fill" style="width: {progress * 100}%"></span>
			</button>
			<span class="clock">{formatTime(status.track?.durationMs ?? 0)}</span>
		</div>
	{/if}
	<div class="player-controls">
		<button class="play" onclick={() => api.togglePlay().then(refreshStatus)} title="Play / pause (Space)">
			<span class="ink-mark">{status.playing ? 'PAUSE' : 'PLAY'}</span>
		</button>
	</div>
</footer>

{/if}

{#if scan}
	<div class="toast"><span>SCAN</span></div>
{:else if toast}
	<div class="toast"><span>{toast}</span></div>
{/if}

{#if popup === 'actions' && target}
	<Popup onclose={closePopup}>
		<p class="pop-text">{target.track.title}</p>
		<div class="pop-divider"></div>
		<div class="menu">
			{#each actions as action, i}
				<button class="menu-item" onclick={() => action.run()} onmouseenter={() => (selectedAction = i)}>
					<span class:ink-mark={i === selectedAction}>{action.label}</span>
				</button>
			{/each}
		</div>
	</Popup>
{:else if popup === 'queue'}
	<Popup onclose={closePopup}>
		<p class="pop-heading">UP NEXT</p>
		<div class="pop-divider"></div>
		{#if upcoming.length}
			<div class="queue scroll" use:scrollFade>
				{#each upcoming as t, i (`${t.id}-${i}`)}
					<p class="queue-row"><span class="no">{i + 1}</span><span class="name">{t.title}</span><span class="artist">{t.artist}</span></p>
				{/each}
			</div>
		{:else}
			<p class="pop-note">Nothing after this song.</p>
		{/if}
	</Popup>
{:else if popup === 'info'}
	<Popup onclose={closePopup}>
		<p class="pop-heading">RE:PLAY</p>
		<div class="pop-divider"></div>
		<p class="pop-text">{allTracks.length} {allTracks.length === 1 ? 'song' : 'songs'}</p>
		<p class="pop-note">{folder ?? 'No folder chosen'}</p>
		<div class="pop-actions">
			<button class="pop-btn" onclick={chooseFolder}>FOLDER</button>
			<button class="pop-btn" onclick={() => { api.rescan(); closePopup(); }}>RESCAN</button>
		</div>
		<div class="pop-divider"></div>
		<div class="shortcuts">
			{#each shortcuts as [key, desc]}
				<span class="key">{key}</span><span class="desc">{desc}</span>
			{/each}
		</div>
		<div class="pop-divider"></div>
		<p class="pop-footer">MP3 · AAC · ALAC · FLAC · WAV · AIFF · OGG<br />MP4 · MOV · WEBM</p>
	</Popup>
{/if}

{#if video}
	<Video paths={video.paths} start={video.start} onclose={() => (video = null)} />
{/if}

<style>
	main {
		position: fixed;
		inset: 0;
	}

	/* ── side buttons ── */
	.shuf { left: calc(6.25% - 28px); top: calc(6.25% + 8px); }
	.rept { right: calc(6.25% - 28px); top: calc(6.25% + 8px); }
	.nav-prev { left: calc(6.25% - 28px); top: 50%; transform: translateY(-50%); }
	.nav-next { right: calc(6.25% - 28px); top: 50%; transform: translateY(-50%); }
	.nav-info { left: calc(6.25% - 28px); bottom: 6.25%; width: 42px; }
	.nav-mini { right: calc(6.25% - 28px); bottom: 6.25%; width: 42px; }

	/* ── search, in the place of a title ── */
	.search {
		position: fixed;
		top: 32px;
		left: 50%;
		transform: translateX(-50%);
		width: 360px;
		max-width: calc(100% - 2 * (6.25% + 60px));
		background: transparent;
		border: none;
		outline: none;
		font-family: 'Kurale', serif;
		font-size: 24px;
		line-height: 32px;
		text-align: center;
		color: var(--text);
		z-index: 5;
	}

	.search::placeholder {
		color: var(--muted);
		opacity: 0.5;
	}

	/* ── radio: genres | stations ── */
	.radio {
		position: absolute;
		top: calc(6.25% + 40px);
		bottom: calc(6.25% + 132px);
		left: calc(6.25% + 96px);
		right: calc(6.25% + 96px);
		display: grid;
		grid-template-columns: 200px minmax(0, 1fr);
		column-gap: 48px;
	}

	.genres,
	.stations {
		overflow-y: auto;
		overflow-x: hidden;
		display: flex;
		flex-direction: column;
		gap: 16px;
		padding: 0.4em 0.6em;
	}

	.genres .name { opacity: 0.6; }
	.genres .name.chosen { opacity: 1; }
	.genres.dim { opacity: 0.35; }

	.station-row {
		display: grid;
		grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) 80px;
		column-gap: 24px;
		width: 100%;
	}

	.kind {
		font-size: 13px;
		opacity: 0.45;
		text-align: right;
	}

	.kept {
		font-size: 9px;
		margin-left: 8px;
		opacity: 0.55;
	}

	/* ── songs ── */
	.songs {
		position: absolute;
		/* right under the search */
		top: calc(6.25% + 40px);
		bottom: calc(6.25% + 132px);
		left: calc(6.25% + 96px);
		right: calc(6.25% + 96px);
		overflow-y: auto;
		overflow-x: hidden;
		display: flex;
		flex-direction: column;
		gap: 16px;
		/* room for the ink stroke of the selected row, which the scroll box would clip */
		padding: 0.4em 0.6em;
	}

	.row {
		display: flex;
		align-items: baseline;
		gap: 14px;
		min-width: 0;
		flex-shrink: 0;
		background: transparent;
		border: none;
		padding: 0;
		text-align: left;
		font-family: 'Kurale', serif;
		font-size: 16px;
		line-height: 1.2;
		cursor: pointer;
		scroll-margin: 56px 0;
	}

	.name {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
		min-width: 0;
	}

	.time,
	.clock {
		font-size: 15px;
		opacity: 0.6;
		font-variant-numeric: tabular-nums;
		flex-shrink: 0;
	}

	/* the song that is playing */
	.dot {
		display: inline-block;
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--text);
		margin-right: 4px;
		vertical-align: middle;
		opacity: 1;
	}

	.song-row {
		display: grid;
		grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) 60px;
		column-gap: 24px;
		width: 100%;
	}

	.cell-title {
		display: flex;
		align-items: baseline;
		min-width: 0;
	}

	.cell {
		font-size: 16px;
		opacity: 0.7;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.cell.time { text-align: right; }

	.empty {
		font-family: 'Kurale', serif;
		font-size: 16px;
		opacity: 0.45;
	}

	.welcome {
		position: absolute;
		top: 45%;
		left: 50%;
		transform: translate(-50%, -50%);
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 2rem;
		font-family: 'Kurale', serif;
		font-size: 24px;
		text-align: center;
	}

	.welcome .pop-btn,
	.player .play {
		cursor: pointer;
	}

	/* ── player ── */
	.player {
		position: fixed;
		left: 50%;
		bottom: calc(6.25% - 14px);
		transform: translateX(-50%);
		width: min(560px, calc(100% - 2 * (6.25% + 100px)));
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 10px;
		z-index: 5;
	}

	.now {
		max-width: 100%;
		font-family: 'Kurale', serif;
		font-size: 24px;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.now-artist { opacity: 0.6; }
	.now.idle { opacity: 0.45; }
	.now.error { font-size: 15px; color: var(--error); }

	.progress-row {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 14px;
	}

	.bar {
		position: relative;
		flex: 1;
		height: 18px;
		background: transparent;
		border: none;
		padding: 0;
		cursor: pointer;
	}

	.bar-line,
	.bar-fill {
		position: absolute;
		left: 0;
		top: 50%;
		height: 2px;
		transform: translateY(-50%);
		background: var(--line);
	}

	.bar-line { right: 0; opacity: 0.25; }
	.bar-fill { transition: width 400ms linear; }

	.play {
		background: transparent;
		border: none;
		padding: 0.3em 0.6em;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 15px;
		letter-spacing: 0.4em;
	}

	.play .ink-mark { padding-right: calc(0.45em - 0.4em); }

	.player-controls {
		position: relative;
		display: flex;
		justify-content: center;
		width: 100%;
	}

	.mini-btn,
	.mini-expand {
		background: transparent;
		border: none;
		padding: 0.3em 0.4em;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 12px;
		letter-spacing: 0.3em;
		opacity: 0.6;
		cursor: pointer;
		transition: opacity 200ms ease;
	}

	.mini-btn:hover,
	.mini-expand:hover {
		opacity: 1;
	}

	/* only in the window in use: in one behind, LIBRARY in the corner stays out of sight */
	:global(html.blurred) .mini-expand {
		opacity: 0 !important;
		pointer-events: none;
	}

	.mini-btn.arrow {
		font-size: 20px;
		letter-spacing: 0;
		line-height: 1;
	}

	/* ── mini player ── */
	.mini {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 10px;
		padding: 36px 28px 24px;
	}

	.mini-expand {
		position: fixed;
		top: 12px;
		right: 12px;
	}

	.mini-title {
		max-width: 100%;
		font-family: 'Kurale', serif;
		font-size: 24px;
		line-height: 1.25;
		text-align: center;
		overflow-wrap: anywhere;
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.mini-title.idle { opacity: 0.45; }
	.mini-title.error { font-size: 15px; color: var(--error); }

	.mini-artist {
		max-width: 100%;
		font-family: 'Kurale', serif;
		font-size: 16px;
		opacity: 0.6;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.mini-progress {
		margin-top: 14px;
		max-width: 380px;
	}

	.mini-controls {
		display: flex;
		align-items: center;
		gap: 1.6rem;
		margin-top: 4px;
	}

	/* ── toast (like ReNote's SAVED) ── */
	.toast {
		position: fixed;
		top: 0;
		left: 50%;
		transform: translateX(-50%);
		width: 42px;
		padding: 1.5rem 0 calc(1.5rem - 0.6em);
		border: 2px solid var(--line);
		border-top: none;
		background: var(--bg) url('/noise.svg') repeat;
		display: flex;
		justify-content: center;
		z-index: 80;
	}

	.toast span {
		writing-mode: vertical-rl;
		text-orientation: upright;
		letter-spacing: 0.6em;
		font-size: 16px;
		line-height: 32px;
	}

	/* ── popups ── */
	.menu {
		display: flex;
		flex-direction: column;
		align-items: flex-start;
		gap: 16px;
		padding: 0.3em 0.5em;
		max-height: 50vh;
		overflow-y: auto;
	}

	.menu-item {
		background: transparent;
		border: none;
		padding: 0;
		font-family: 'Kurale', serif;
		font-size: 16px;
		line-height: 1.2;
		text-align: left;
		cursor: pointer;
	}

	.queue {
		width: 100%;
		max-height: 50vh;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.queue-row {
		display: flex;
		align-items: baseline;
		gap: 12px;
		font-family: 'Kurale', serif;
		font-size: 16px;
		min-width: 0;
	}

	.queue-row .no {
		width: 1.8em;
		flex-shrink: 0;
		text-align: right;
		font-size: 14px;
		opacity: 0.55;
	}

	.queue-row .artist {
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 12px;
		letter-spacing: 0.06em;
		opacity: 0.55;
		white-space: nowrap;
		margin-left: auto;
		padding-left: 12px;
	}

	.pop-note {
		font-family: 'Kurale', serif;
		font-size: 15px;
		opacity: 0.65;
		text-align: center;
		overflow-wrap: anywhere;
	}

	.shortcuts {
		display: grid;
		grid-template-columns: auto auto;
		gap: 8px 24px;
		font-size: 13px;
	}

	.shortcuts .key {
		font-family: 'Noto Sans JP', sans-serif;
		letter-spacing: 0.1em;
		text-align: right;
		opacity: 0.85;
	}

	.shortcuts .desc {
		font-family: 'Kurale', serif;
		opacity: 0.7;
	}

	.pop-footer {
		font-family: 'Kurale', serif;
		font-size: 13px;
		letter-spacing: 0.15em;
		opacity: 0.55;
	}

	/* welcome button reuses the popup button look */
	.welcome :global(.pop-btn),
	.welcome .pop-btn {
		background: transparent;
		border: 2px solid var(--line);
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 16px;
		letter-spacing: 0.2em;
		padding: 0.5rem 1.4rem;
		transition: background 200ms ease, color 200ms ease;
	}

	.welcome .pop-btn:hover {
		background: var(--line);
		color: var(--bg);
	}
</style>
