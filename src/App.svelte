<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import WindowControls from './lib/WindowControls.svelte';
	import Popup from './lib/Popup.svelte';
	import { scrollFade } from './lib/scrollFade';
	import { api, songs, matches, formatTime, type Track, type Status } from './lib/music';

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

	let status: Status = $state({ track: null, playing: false, positionMs: 0, shuffle: false, repeat: 'off', error: null });

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

	async function toggleShuffle() {
		await api.setShuffle(!status.shuffle);
		await refreshStatus();
	}

	async function cycleRepeat() {
		await api.cycleRepeat();
		await refreshStatus();
	}

	function seekTo(e: MouseEvent) {
		const t = status.track;
		if (!t) return;
		const bar = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const fraction = Math.min(1, Math.max(0, (e.clientX - bar.left) / bar.width));
		api.seek(fraction * t.durationMs).then(refreshStatus);
	}

	let progress = $derived(status.track && status.track.durationMs ? Math.min(1, status.positionMs / status.track.durationMs) : 0);

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

	// ── keyboard ──

	function handleKeydown(e: KeyboardEvent) {
		const mod = e.metaKey || e.ctrlKey;

		if (mini) {
			if (e.key === ' ') {
				e.preventDefault();
				api.togglePlay().then(refreshStatus);
			} else if (e.key === 'ArrowRight') {
				e.preventDefault();
				api.next().then(refreshStatus);
			} else if (e.key === 'ArrowLeft') {
				e.preventDefault();
				api.previous().then(refreshStatus);
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

		if (mod && e.key === 'ArrowRight') {
			e.preventDefault();
			api.next().then(refreshStatus);
		} else if (mod && e.key === 'ArrowLeft') {
			e.preventDefault();
			api.previous().then(refreshStatus);
		} else if (e.key === ' ') {
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
		const unlisten = [
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
		['Space', 'Play / pause'],
		['⌘ ← / ⌘ →', 'Previous / next song'],
		['↑ ↓', 'Move around'],
		['Enter', 'Play'],
		['/', 'Search'],
		['S / R', 'Shuffle / repeat'],
		['Q', 'Up next'],
		['E / right click', 'Actions'],
		['M', 'Mini player'],
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
		{:else if status.track}
			<p class="mini-title">{status.track.title}</p>
			<p class="mini-artist">{status.track.artist || status.track.albumArtist}</p>
		{:else}
			<p class="mini-title idle">Nothing playing.</p>
		{/if}
		<div class="progress-row mini-progress">
			<span class="clock">{formatTime(status.positionMs)}</span>
			<button class="bar" onclick={seekTo} aria-label="Seek" disabled={!status.track}>
				<span class="bar-line"></span>
				<span class="bar-fill" style="width: {progress * 100}%"></span>
			</button>
			<span class="clock">{formatTime(status.track?.durationMs ?? 0)}</span>
		</div>
		<div class="mini-controls">
			<button class="mini-btn" onclick={() => api.previous().then(refreshStatus)} title="Previous (←)">PREV</button>
			<button class="play" onclick={() => api.togglePlay().then(refreshStatus)} title="Play / pause (Space)">
				<span class="ink-mark">{status.playing ? 'PAUSE' : 'PLAY'}</span>
			</button>
			<button class="mini-btn" onclick={() => api.next().then(refreshStatus)} title="Next (→)">NEXT</button>
		</div>
	</main>
{:else}

<!-- shuffle / repeat: inked with a vertical stroke when on -->
<button class="side switch shuf" class:ink-mark-v={status.shuffle} onclick={toggleShuffle} title="Shuffle (S)">SHUF</button>
<button class="side switch rept" class:ink-mark-v={status.repeat !== 'off'} onclick={cycleRepeat} title="Repeat (R)">
	{status.repeat === 'one' ? 'ONE' : 'REPT'}
</button>

<button class="side boxed nav-prev" onclick={() => api.previous().then(refreshStatus)} title="Previous (⌘←)">PREV</button>
<button class="side boxed nav-next" onclick={() => api.next().then(refreshStatus)} title="Next (⌘→)">NEXT</button>

<button class="side plain nav-info" onclick={() => (popup = 'info')} title="Info (I)">INFO</button>
<button class="side plain nav-mini" onclick={() => setMini(true)} title="Mini player (M)">MINI</button>

<!-- the search is the title -->
<input bind:this={searchInput} bind:value={query} class="search" type="text" placeholder="Search" oninput={() => (ti = 0)} />

<main class="fade-in">
	{#if loaded && allTracks.length === 0 && !scan}
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
	{:else if status.track}
		<p class="now"><span class="now-title">{status.track.title}</span><span class="now-artist"> — {status.track.artist || status.track.albumArtist}</span></p>
	{:else}
		<p class="now idle">Nothing playing.</p>
	{/if}
	<div class="progress-row">
		<span class="clock">{formatTime(status.positionMs)}</span>
		<button class="bar" onclick={seekTo} aria-label="Seek" disabled={!status.track}>
			<span class="bar-line"></span>
			<span class="bar-fill" style="width: {progress * 100}%"></span>
		</button>
		<span class="clock">{formatTime(status.track?.durationMs ?? 0)}</span>
	</div>
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
		<p class="pop-heading">RE:SOUND</p>
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
		<p class="pop-footer">MP3 · AAC · ALAC · FLAC · WAV · AIFF · OGG</p>
	</Popup>
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
		cursor: url('/ink/hand.svg'), pointer;
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
		cursor: url('/ink/hand.svg'), pointer;
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
	.now.error { font-size: 15px; color: #b04a45; }

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
		cursor: url('/ink/hand.svg'), pointer;
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
		cursor: url('/ink/hand.svg'), pointer;
		transition: opacity 200ms ease;
	}

	.mini-btn:hover,
	.mini-expand:hover {
		opacity: 1;
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
	.mini-title.error { font-size: 15px; color: #b04a45; }

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
		cursor: url('/ink/hand.svg'), pointer;
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
