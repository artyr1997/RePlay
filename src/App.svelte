<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import WindowControls from './lib/WindowControls.svelte';
	import Popup from './lib/Popup.svelte';
	import Cover from './lib/Cover.svelte';
	import { scrollFade } from './lib/scrollFade';
	import {
		api, albums, artists, songs, matches, formatTime,
		type Track, type Playlist, type Group, type Status,
	} from './lib/music';

	type Tab = 'albums' | 'artists' | 'songs' | 'lists';
	type Focus = 'groups' | 'tracks';
	type Target =
		| { kind: 'track'; track: Track; index: number }
		| { kind: 'group'; group: Group }
		| { kind: 'playlist'; playlist: Playlist };
	type PopupKind = 'none' | 'actions' | 'pick-playlist' | 'name' | 'delete' | 'queue' | 'info';

	const tabs: { id: Tab; label: string }[] = [
		{ id: 'albums', label: 'ALBUMS' },
		{ id: 'artists', label: 'ARTISTS' },
		{ id: 'songs', label: 'SONGS' },
		{ id: 'lists', label: 'LISTS' },
	];

	let tab = $state<Tab>('albums');
	let focus = $state<Focus>('groups');
	let allTracks: Track[] = $state([]);
	let lists: Playlist[] = $state([]);
	let listTracks: Track[] = $state([]);
	let gi = $state(0);
	let ti = $state(0);
	let query = $state('');
	let loaded = $state(false);
	let folder: string | null = $state(null);
	let scan: { done: number; total: number } | null = $state(null);

	let status: Status = $state({ track: null, playing: false, positionMs: 0, shuffle: false, repeat: 'off', error: null });

	let searchInput: HTMLInputElement | undefined = $state();
	let groupList: HTMLDivElement | undefined = $state();
	let trackList: HTMLDivElement | undefined = $state();

	// ── what is shown ──

	let q = $derived(query.trim().toLowerCase());
	let filtered = $derived(q ? allTracks.filter(t => matches(t, q)) : allTracks);

	let groups: Group[] = $derived.by(() => {
		if (tab === 'albums') return albums(filtered);
		if (tab === 'artists') return artists(filtered);
		if (tab === 'lists') {
			return lists
				.filter(p => !q || p.name.toLowerCase().includes(q))
				.map(p => ({ key: String(p.id), title: p.name, subtitle: `${p.trackCount} ${p.trackCount === 1 ? 'song' : 'songs'}`, tracks: [] }));
		}
		return [];
	});
	let group: Group | undefined = $derived(groups[gi]);
	let playlist: Playlist | undefined = $derived(tab === 'lists' && group ? lists.find(p => String(p.id) === group.key) : undefined);

	let tracks: Track[] = $derived(
		tab === 'songs' ? songs(filtered) : tab === 'lists' ? listTracks : (group?.tracks ?? []),
	);
	let track: Track | undefined = $derived(tracks[ti]);

	// playlist contents load when the selection changes
	let listToken = 0;
	$effect(() => {
		const p = playlist;
		const token = ++listToken;
		if (!p) {
			listTracks = [];
			return;
		}
		api.playlistTracks(p.id).then(t => {
			if (token === listToken) listTracks = t;
		});
	});

	const albumCount = $derived(albums(allTracks).length);
	const artistCount = $derived(artists(allTracks).length);

	// ── loading ──

	async function loadLibrary() {
		[allTracks, lists, folder] = await Promise.all([api.tracks(), api.playlists(), api.musicFolder()]);
		loaded = true;
		gi = Math.min(gi, Math.max(groups.length - 1, 0));
		ti = Math.min(ti, Math.max(tracks.length - 1, 0));
	}

	async function reloadLists() {
		lists = await api.playlists();
		if (playlist) listTracks = await api.playlistTracks(playlist.id);
	}

	function selectTab(next: Tab) {
		if (tab === next) return;
		tab = next;
		gi = 0;
		ti = 0;
		focus = next === 'songs' ? 'tracks' : 'groups';
	}

	// ── playback ──

	async function playTracks(list: Track[], start = 0, shuffle = false) {
		if (!list.length) return;
		if (shuffle !== status.shuffle) await api.setShuffle(shuffle);
		await api.play(list.map(t => t.id), shuffle ? Math.floor(Math.random() * list.length) : start);
		await refreshStatus();
	}

	async function tracksOf(target: Target): Promise<Track[]> {
		if (target.kind === 'track') return [target.track];
		if (target.kind === 'group') return target.group.tracks.length ? target.group.tracks : tracks;
		return api.playlistTracks(target.playlist.id);
	}

	function playSelected() {
		if (focus === 'tracks' || tab === 'songs') playTracks(tracks, ti);
		else if (tab === 'lists') playTracks(tracks, 0);
		else if (group) playTracks(group.tracks, 0);
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

	async function scrollIntoView(list: HTMLDivElement | undefined) {
		await tick();
		list?.querySelector('.row.active')?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
	}

	function move(delta: number) {
		if (focus === 'groups' && tab !== 'songs') {
			if (!groups.length) return;
			gi = (gi + delta + groups.length) % groups.length;
			ti = 0;
			scrollIntoView(groupList);
		} else {
			if (!tracks.length) return;
			ti = (ti + delta + tracks.length) % tracks.length;
			scrollIntoView(trackList);
		}
	}

	// ── popups ──

	let popup = $state<PopupKind>('none');
	let target: Target | null = $state(null);
	let selectedAction = $state(0);
	let pickIndex = $state(0);
	let nameMode = $state<'create' | 'rename'>('create');
	let nameValue = $state('');
	let nameInput: HTMLInputElement | undefined = $state();
	let pendingTrackIds: number[] = $state([]);
	let popupError = $state('');
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
			await fn((await tracksOf(t)).map(x => x.id));
			closePopup();
			showToast(msg);
		};
		const common: Action[] = [
			{ label: 'Play next', run: () => add(api.playNext, 'NEXT') },
			{ label: 'Add to queue', run: () => add(api.addToQueue, 'QUEUED') },
			{ label: 'Add to playlist…', run: () => openPicker(t) },
		];
		if (t.kind === 'track') {
			return [
				{ label: 'Play', run: () => { closePopup(); playTracks(tracks, t.index); } },
				...common,
				...(tab === 'lists' && playlist ? [{ label: 'Remove from playlist', run: () => removeFromPlaylist(t.index) }] : []),
				{ label: 'Show in Finder', run: () => { closePopup(); api.reveal(t.track.path); } },
			];
		}
		if (t.kind === 'group') {
			return [
				{ label: 'Play', run: () => { closePopup(); playTracks(t.group.tracks, 0); } },
				{ label: 'Shuffle', run: () => { closePopup(); playTracks(t.group.tracks, 0, true); } },
				...common,
			];
		}
		return [
			{ label: 'Play', run: async () => { closePopup(); playTracks(await tracksOf(t), 0); } },
			{ label: 'Shuffle', run: async () => { closePopup(); playTracks(await tracksOf(t), 0, true); } },
			{ label: 'Rename', run: () => openName('rename', t.playlist.name) },
			{ label: 'Delete', run: () => { popupError = ''; popup = 'delete'; } },
		];
	});

	function currentTarget(): Target | null {
		if ((focus === 'tracks' || tab === 'songs') && track) return { kind: 'track', track, index: ti };
		if (tab === 'lists' && playlist) return { kind: 'playlist', playlist };
		if (group) return { kind: 'group', group };
		return null;
	}

	function openActions(t: Target | null = currentTarget()) {
		if (!t) return;
		target = t;
		selectedAction = 0;
		popupError = '';
		popup = 'actions';
	}

	async function openPicker(t: Target) {
		pendingTrackIds = (await tracksOf(t)).map(x => x.id);
		pickIndex = 0;
		popupError = '';
		popup = 'pick-playlist';
	}

	async function pickPlaylist(index: number) {
		if (index === 0) {
			openName('create');
			return;
		}
		const p = lists[index - 1];
		await api.addToPlaylist(p.id, pendingTrackIds);
		closePopup();
		await reloadLists();
		showToast('ADDED');
	}

	async function openName(mode: 'create' | 'rename', value = '') {
		nameMode = mode;
		nameValue = value;
		popupError = '';
		popup = 'name';
		await tick();
		nameInput?.focus();
		nameInput?.select();
	}

	async function submitName() {
		try {
			if (nameMode === 'create') {
				const id = await api.createPlaylist(nameValue, pendingTrackIds);
				pendingTrackIds = [];
				await reloadLists();
				if (tab === 'lists') {
					query = '';
					gi = Math.max(0, lists.findIndex(p => p.id === id));
				}
				showToast('SAVED');
			} else if (target?.kind === 'playlist') {
				await api.renamePlaylist(target.playlist.id, nameValue);
				await reloadLists();
			}
			closePopup();
		} catch (e) {
			popupError = String(e);
		}
	}

	async function deletePlaylist() {
		if (target?.kind !== 'playlist') return;
		await api.deletePlaylist(target.playlist.id);
		closePopup();
		await reloadLists();
		gi = Math.min(gi, Math.max(groups.length - 1, 0));
	}

	async function removeFromPlaylist(index: number) {
		if (!playlist) return;
		await api.removeFromPlaylist(playlist.id, index);
		closePopup();
		await reloadLists();
		ti = Math.min(ti, Math.max(listTracks.length - 1, 0));
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

		if (popup !== 'none') {
			if (e.key === 'Escape') {
				e.preventDefault();
				closePopup();
				return;
			}
			const listLen = popup === 'actions' ? actions.length : popup === 'pick-playlist' ? lists.length + 1 : 0;
			if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && listLen) {
				e.preventDefault();
				const d = e.key === 'ArrowDown' ? 1 : -1;
				if (popup === 'actions') selectedAction = (selectedAction + d + listLen) % listLen;
				else pickIndex = (pickIndex + d + listLen) % listLen;
			} else if (e.key === 'Enter') {
				e.preventDefault();
				if (popup === 'actions') actions[selectedAction]?.run();
				else if (popup === 'pick-playlist') pickPlaylist(pickIndex);
				else if (popup === 'name') submitName();
				else if (popup === 'delete') deletePlaylist();
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
				playSelected();
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
		} else if (e.key === 'ArrowRight' && tab !== 'songs') {
			e.preventDefault();
			if (tracks.length) focus = 'tracks';
		} else if (e.key === 'ArrowLeft' && tab !== 'songs') {
			e.preventDefault();
			focus = 'groups';
		} else if (e.key === 'Enter') {
			e.preventDefault();
			playSelected();
		} else if (e.key === 'Backspace' || e.key === 'Delete') {
			e.preventDefault();
			if (tab === 'lists' && focus === 'tracks' && track) removeFromPlaylist(ti);
			else if (tab === 'lists' && playlist) {
				target = { kind: 'playlist', playlist };
				popup = 'delete';
			}
		} else if (e.code === 'Slash') {
			e.preventDefault();
			searchInput?.focus();
		} else if (!mod && /^Digit[1-4]$/.test(e.code)) {
			selectTab(tabs[Number(e.code.slice(5)) - 1].id);
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
		} else if (!mod && e.code === 'KeyN') {
			pendingTrackIds = [];
			openName('create');
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
		['↑ ↓ ← →', 'Move around'],
		['Enter', 'Play'],
		['1 – 4', 'Albums, Artists, Songs, Lists'],
		['/', 'Search'],
		['S / R', 'Shuffle / repeat'],
		['Q', 'Up next'],
		['E / right click', 'Actions'],
		['N', 'New playlist'],
		['I', 'Info'],
	];
</script>

<svelte:window
	onkeydown={handleKeydown}
	oncontextmenu={e => {
		if ((e.target as HTMLElement)?.tagName !== 'INPUT') e.preventDefault();
	}}
/>

<WindowControls />

<!-- shuffle / repeat: inked with a vertical stroke when on -->
<button class="side switch shuf" class:ink-mark-v={status.shuffle} onclick={toggleShuffle} title="Shuffle (S)">SHUF</button>
<button class="side switch rept" class:ink-mark-v={status.repeat !== 'off'} onclick={cycleRepeat} title="Repeat (R)">
	{status.repeat === 'one' ? 'ONE' : 'REPT'}
</button>

<button class="side boxed nav-prev" onclick={() => api.previous().then(refreshStatus)} title="Previous (⌘←)">PREV</button>
<button class="side boxed nav-next" onclick={() => api.next().then(refreshStatus)} title="Next (⌘→)">NEXT</button>

<button class="side plain nav-info" onclick={() => (popup = 'info')} title="Info (I)">INFO</button>
<button class="side plain nav-queue" onclick={openQueue} title="Up next (Q)">QUEUE</button>

<nav class="tabs">
	{#each tabs as t}
		<button class="tab" onclick={() => selectTab(t.id)}>
			<span class:ink-mark={tab === t.id}>{t.label}</span>
		</button>
	{/each}
</nav>

<main class="fade-in">
	{#if loaded && allTracks.length === 0 && !scan}
		<div class="welcome">
			<p>No music in <span class="ink-mark">{folder ? folder.split('/').pop() : 'your folder'}</span> yet.</p>
			<button class="pop-btn" onclick={chooseFolder}>CHOOSE FOLDER</button>
		</div>
	{:else if tab === 'songs'}
		<section class="songs">
			<input bind:this={searchInput} bind:value={query} class="search" type="text" placeholder="Search" oninput={() => (ti = 0)} />
			<div class="list scroll" bind:this={trackList} use:scrollFade>
				{#each tracks as t, i (t.id)}
					<button
						class="row song-row"
						class:active={i === ti}
						onclick={() => (ti = i)}
						ondblclick={() => playTracks(tracks, i)}
						oncontextmenu={() => { ti = i; openActions({ kind: 'track', track: t, index: i }); }}
					>
						<span class="cell-title">
							{#if status.track?.id === t.id}<span class="dot"></span>{/if}
							<span class="name" class:ink-mark={i === ti}>{t.title}</span>
						</span>
						<span class="cell">{t.artist}</span>
						<span class="cell">{t.album}</span>
						<span class="cell time">{formatTime(t.durationMs)}</span>
					</button>
				{:else}
					<p class="empty">{q ? 'Nothing found.' : 'No songs yet.'}</p>
				{/each}
			</div>
		</section>
	{:else}
		<section class="groups">
			<input bind:this={searchInput} bind:value={query} class="search" type="text" placeholder="Search" oninput={() => { gi = 0; ti = 0; }} />
			<div class="list scroll" bind:this={groupList} use:scrollFade>
				{#each groups as g, i (g.key)}
					<button
						class="row group-row"
						class:active={i === gi}
						onclick={() => { gi = i; ti = 0; focus = 'groups'; }}
						ondblclick={() => { gi = i; playSelected(); }}
						oncontextmenu={() => {
							gi = i;
							const p = tab === 'lists' ? lists.find(x => String(x.id) === g.key) : undefined;
							openActions(p ? { kind: 'playlist', playlist: p } : { kind: 'group', group: g });
						}}
					>
						<span class="name" class:ink-mark={i === gi}>{g.title}</span>
						<span class="sub">{g.subtitle}</span>
					</button>
				{:else}
					<p class="empty">
						{#if q}Nothing found.{:else if tab === 'lists'}No playlists yet. Press <b>N</b>.{:else}No music yet.{/if}
					</p>
				{/each}
			</div>
		</section>

		<section class="tracks">
			{#if group}
				{#key group.key}
					<header class="head fade-in">
						{#if tab === 'albums' && group.tracks[0]}
							<Cover trackId={group.tracks[0].id} title={group.title} size={112} />
						{/if}
						<div class="head-text">
							<p class="head-title">{group.title}</p>
							<p class="head-sub">
								{group.subtitle}{#if tab === 'albums' && group.tracks[0]?.year} · {group.tracks[0].year}{/if}
								· {formatTime(tracks.reduce((s, t) => s + t.durationMs, 0))}
							</p>
						</div>
					</header>
				{/key}
			{/if}
			<div class="list scroll" bind:this={trackList} use:scrollFade>
				{#each tracks as t, i (`${t.id}-${i}`)}
					<button
						class="row track-row"
						class:active={i === ti && focus === 'tracks'}
						onclick={() => { ti = i; focus = 'tracks'; }}
						ondblclick={() => playTracks(tracks, i)}
						oncontextmenu={() => { ti = i; focus = 'tracks'; openActions({ kind: 'track', track: t, index: i }); }}
					>
						<span class="no">
							{#if status.track?.id === t.id}<span class="dot"></span>{:else if tab === 'albums'}{t.trackNo ?? ''}{:else}{i + 1}{/if}
						</span>
						<span class="name" class:ink-mark={i === ti && focus === 'tracks'}>{t.title}</span>
						{#if tab !== 'albums'}<span class="artist">{t.artist}</span>{/if}
						<span class="time">{formatTime(t.durationMs)}</span>
					</button>
				{:else}
					{#if tab === 'lists' && playlist}<p class="empty">Empty playlist. Add songs with <b>E</b>.</p>{/if}
				{/each}
			</div>
		</section>
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
	<button class="play" onclick={() => api.togglePlay().then(refreshStatus)} title="Play / pause (Space)">
		<span class="ink-mark">{status.playing ? 'PAUSE' : 'PLAY'}</span>
	</button>
</footer>

{#if scan}
	<div class="toast"><span>SCAN</span></div>
{:else if toast}
	<div class="toast"><span>{toast}</span></div>
{/if}

{#if popup === 'actions' && target}
	<Popup onclose={closePopup}>
		<p class="pop-text">
			{target.kind === 'track' ? target.track.title : target.kind === 'group' ? target.group.title : target.playlist.name}
		</p>
		<div class="pop-divider"></div>
		<div class="menu">
			{#each actions as action, i}
				<button class="menu-item" onclick={() => action.run()} onmouseenter={() => (selectedAction = i)}>
					<span class:ink-mark={i === selectedAction}>{action.label}</span>
				</button>
			{/each}
		</div>
	</Popup>
{:else if popup === 'pick-playlist'}
	<Popup onclose={closePopup}>
		<p class="pop-heading">ADD TO PLAYLIST</p>
		<div class="pop-divider"></div>
		<div class="menu">
			{#each ['New playlist…', ...lists.map(p => p.name)] as label, i}
				<button class="menu-item" onclick={() => pickPlaylist(i)} onmouseenter={() => (pickIndex = i)}>
					<span class:ink-mark={i === pickIndex}>{label}</span>
				</button>
			{/each}
		</div>
	</Popup>
{:else if popup === 'name'}
	<Popup onclose={closePopup}>
		<p class="pop-heading">{nameMode === 'create' ? 'NEW PLAYLIST' : 'RENAME'}</p>
		<div class="pop-divider"></div>
		<input bind:this={nameInput} bind:value={nameValue} class="name-input" type="text" maxlength="80" placeholder="Name" />
		{#if popupError}<p class="pop-error">{popupError}</p>{/if}
		<div class="pop-actions">
			<button class="pop-btn" onclick={submitName}>{nameMode === 'create' ? 'CREATE' : 'RENAME'}</button>
			<button class="pop-btn" onclick={closePopup}>CANCEL</button>
		</div>
	</Popup>
{:else if popup === 'delete' && target?.kind === 'playlist'}
	<Popup onclose={closePopup}>
		<p class="pop-text">Delete the playlist “{target.playlist.name}”?</p>
		<p class="pop-note">The songs stay in your library.</p>
		<div class="pop-actions">
			<button class="pop-btn" onclick={deletePlaylist}>YES</button>
			<button class="pop-btn" onclick={closePopup}>NO</button>
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
		<p class="pop-text">{allTracks.length} songs · {albumCount} albums · {artistCount} artists</p>
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
	.nav-queue { right: calc(6.25% - 28px); bottom: 6.25%; width: 42px; }

	/* ── tabs ── */
	.tabs {
		position: fixed;
		top: calc(6.25% + 2px);
		left: 50%;
		transform: translateX(-50%);
		display: flex;
		gap: 2.2rem;
		z-index: 5;
	}

	.tab {
		background: transparent;
		border: none;
		padding: 0.3em 0.5em;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 14px;
		letter-spacing: 0.35em;
		cursor: url('/ink/hand.svg'), pointer;
	}

	/* ── columns ── */
	.groups,
	.tracks,
	.songs {
		position: absolute;
		top: calc(6.25% + 72px);
		bottom: calc(6.25% + 132px);
		display: flex;
		flex-direction: column;
		gap: 18px;
	}

	.groups { left: calc(21.3% - 0.6em); width: 27%; }
	.tracks { left: 54%; right: calc(6.25% + 64px); }
	.songs { left: calc(6.25% + 96px); right: calc(6.25% + 96px); }

	.search {
		width: 100%;
		background: transparent;
		border: none;
		outline: none;
		font-family: 'Kurale', serif;
		font-size: 24px;
		line-height: 32px;
		text-align: center;
		color: var(--text);
		flex-shrink: 0;
	}

	.search::placeholder {
		color: var(--muted);
		opacity: 0.5;
	}

	.list {
		flex: 1;
		min-height: 0;
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
		font-size: 18px;
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

	.group-row {
		flex-direction: column;
		align-items: flex-start;
		gap: 4px;
	}

	.group-row .name { max-width: 100%; }

	.sub,
	.artist {
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 12px;
		letter-spacing: 0.06em;
		opacity: 0.55;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.track-row .name { flex: 0 1 auto; }
	.track-row .artist { flex: 1 1 0; min-width: 0; }
	.track-row .time { margin-left: auto; padding-left: 12px; }

	.no {
		width: 1.8em;
		flex-shrink: 0;
		text-align: right;
		font-size: 14px;
		opacity: 0.55;
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

	.no .dot { margin: 0; }

	.head {
		display: flex;
		align-items: center;
		gap: 22px;
		padding: 0 0.6em;
		flex-shrink: 0;
	}

	.head-text { min-width: 0; }

	.head-title {
		font-family: 'Kurale', serif;
		font-size: 26px;
		line-height: 1.2;
		overflow-wrap: anywhere;
	}

	.head-sub {
		margin-top: 6px;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 12px;
		letter-spacing: 0.08em;
		opacity: 0.6;
	}

	.song-row {
		display: grid;
		grid-template-columns: minmax(0, 2fr) minmax(0, 1.2fr) minmax(0, 1.2fr) 60px;
		column-gap: 24px;
		width: 100%;
	}

	.cell-title {
		display: flex;
		align-items: baseline;
		min-width: 0;
	}

	.cell {
		font-size: 15px;
		opacity: 0.7;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.cell.time { text-align: right; }

	.empty {
		font-family: 'Kurale', serif;
		font-size: 18px;
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
		font-size: 20px;
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
		font-size: 19px;
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
		font-size: 17px;
		min-width: 0;
	}

	.queue-row .artist { margin-left: auto; padding-left: 12px; }

	.name-input {
		width: 100%;
		background: transparent;
		border: none;
		outline: none;
		font-family: 'Kurale', serif;
		font-size: 24px;
		text-align: center;
		color: var(--text);
	}

	.pop-note {
		font-family: 'Kurale', serif;
		font-size: 15px;
		opacity: 0.65;
		text-align: center;
		overflow-wrap: anywhere;
	}

	.pop-error {
		font-family: 'Kurale', serif;
		font-size: 15px;
		color: #b04a45;
		text-align: center;
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
