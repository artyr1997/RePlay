<script lang="ts">
	// A video over the library. In the window: the picture on the paper, its corners round, and under
	// it, as under the music, its name, the line and the controls, never over the picture. Full screen:
	// the picture alone on black, the controls over its foot on a shade, gone while it plays and the
	// mouse rests. The file comes through Tauri's asset protocol, so WebKit plays it itself:
	// MP4 / M4V / MOV (H.264, HEVC), WebM.
	import { onMount } from 'svelte';
	import { convertFileSrc } from '@tauri-apps/api/core';
	import { getCurrentWindow } from '@tauri-apps/api/window';

	let { paths, start = 0, onclose }: { paths: string[]; start?: number; onclose: () => void } = $props();

	// the first one shown (another video opened is a new Video: App.svelte)
	// svelte-ignore state_referenced_locally
	let index = $state(start);
	let element: HTMLVideoElement | undefined = $state();
	let playing = $state(false);
	let time = $state(0);
	let duration = $state(0);
	let failed = $state(false);
	let resting = $state(false);
	let full = $state(false);
	let restTimer: ReturnType<typeof setTimeout> | null = null;

	// the picture as big as its room lets it, in its own proportions (so its corners are the picture's)
	let roomWidth = $state(0);
	let roomHeight = $state(0);
	let videoWidth = $state(0);
	let videoHeight = $state(0);
	const fit = $derived.by(() => {
		if (!videoWidth || !videoHeight || !roomWidth || !roomHeight) return null;
		const scale = Math.min(roomWidth / videoWidth, roomHeight / videoHeight);
		return { width: Math.floor(videoWidth * scale), height: Math.floor(videoHeight * scale) };
	});

	const path = $derived(paths[index]);
	const name = $derived((path.split('/').pop() ?? path).replace(/\.[^.]+$/, ''));
	const progress = $derived(duration > 0 ? time / duration : 0);

	// a new file: from its start
	$effect(() => {
		path;
		failed = false;
		time = 0;
		duration = 0;
		videoWidth = 0;
		videoHeight = 0;
	});

	function clock(seconds: number) {
		if (!isFinite(seconds) || seconds < 0) seconds = 0;
		const s = Math.floor(seconds % 60).toString().padStart(2, '0');
		const m = Math.floor(seconds / 60) % 60;
		const h = Math.floor(seconds / 3600);
		return h ? `${h}:${m.toString().padStart(2, '0')}:${s}` : `${m}:${s}`;
	}

	function toggle() {
		if (!element) return;
		if (element.paused) element.play().catch(() => {});
		else element.pause();
	}

	function seekBy(seconds: number) {
		if (element) element.currentTime = Math.min(Math.max(element.currentTime + seconds, 0), duration || 0);
	}

	function seekTo(e: MouseEvent) {
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		if (element && duration) element.currentTime = ((e.clientX - rect.left) / rect.width) * duration;
	}

	function step(delta: number) {
		const next = index + delta;
		if (next >= 0 && next < paths.length) index = next;
	}

	const win = getCurrentWindow();
	async function toggleFullscreen() {
		await win.setFullscreen(!(await win.isFullscreen()));
	}

	async function close() {
		if (await win.isFullscreen()) await win.setFullscreen(false);
		onclose();
	}

	// full screen: the controls rest with the mouse while the video plays
	function wake() {
		resting = false;
		if (restTimer) clearTimeout(restTimer);
		restTimer = setTimeout(() => (resting = playing && full), 2200);
	}
	$effect(() => {
		if (!playing || !full) resting = false;
		else wake();
	});

	function keydown(e: KeyboardEvent) {
		if (e.metaKey || e.ctrlKey || e.altKey) return;
		wake();
		if (e.key === ' ' || e.key === 'k') {
			e.preventDefault();
			toggle();
		} else if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
			e.preventDefault();
			const back = e.key === 'ArrowLeft';
			if (e.shiftKey) step(back ? -1 : 1);
			else seekBy(back ? -10 : 10);
		} else if (e.code === 'KeyF') {
			e.preventDefault();
			toggleFullscreen();
		} else if (e.key === 'Escape') {
			e.preventDefault();
			// out of full screen first, then back to the music
			if (full) win.setFullscreen(false);
			else close();
		}
	}

	onMount(() => {
		// the window's own keys while the video is up (RePlay's are held, see App.svelte)
		window.addEventListener('keydown', keydown);
		const look = () => win.isFullscreen().then(f => (full = f));
		look();
		const off = win.onResized(look);
		return () => {
			window.removeEventListener('keydown', keydown);
			off.then(stop => stop());
			if (restTimer) clearTimeout(restTimer);
		};
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="video fade-in" class:full class:resting onmousemove={wake}>
	<div class="room" bind:clientWidth={roomWidth} bind:clientHeight={roomHeight}>
		{#key path}
			<!-- svelte-ignore a11y_media_has_caption -->
			<video
				bind:this={element}
				src={convertFileSrc(path)}
				autoplay
				bind:currentTime={time}
				bind:duration
				bind:videoWidth
				bind:videoHeight
				class:shown={fit}
				style={fit ? `width: ${fit.width}px; height: ${fit.height}px` : ''}
				onplay={() => (playing = true)}
				onpause={() => (playing = false)}
				onended={() => (index < paths.length - 1 ? step(1) : (playing = false))}
				onerror={() => (failed = true)}
				onclick={toggle}
				ondblclick={toggleFullscreen}
			></video>
		{/key}
		{#if failed}
			<p class="failed">This video doesn't play here.</p>
		{/if}
	</div>

	<div class="corner">
		<button class="word" onclick={toggleFullscreen} title="Full screen (F, or a double click)">{full ? 'WINDOW' : 'FULL'}</button>
		<button class="word" onclick={close} title="Back to the music (Esc)">MUSIC</button>
	</div>

	<div class="controls">
		<p class="name">{name}</p>
		<div class="progress-row">
			<span class="clock">{clock(time)}</span>
			<button class="bar" onclick={seekTo} aria-label="Seek">
				<span class="bar-line"></span>
				<span class="bar-fill" style="width: {progress * 100}%"></span>
			</button>
			<span class="clock">{clock(duration)}</span>
		</div>
		<div class="buttons">
			<button class="arrow" onclick={() => step(-1)} disabled={index === 0} title="Previous video (⇧←)" aria-label="Previous">←</button>
			<button class="play" onclick={toggle} title="Play / pause (Space)">
				<span class="ink-mark">{playing ? 'PAUSE' : 'PLAY'}</span>
			</button>
			<button class="arrow" onclick={() => step(1)} disabled={index === paths.length - 1} title="Next video (⇧→)" aria-label="Next">→</button>
		</div>
	</div>
</div>

<style>
	.video {
		position: fixed;
		inset: 0;
		z-index: 50;
		background: var(--bg);
		/* the controls' room under the picture */
		--foot: 150px;
	}

	.video.resting {
		cursor: none;
	}

	/* ── the picture ── */
	.room {
		position: absolute;
		top: 48px;
		left: 56px;
		right: 56px;
		bottom: var(--foot);
		display: flex;
		align-items: center;
		justify-content: center;
	}

	video {
		display: block;
		border-radius: 10px;
		background: transparent;
		opacity: 0;
		transition: opacity 400ms ease;
	}

	video.shown {
		opacity: 1;
	}

	.failed {
		position: absolute;
		font-family: 'Kurale', serif;
		font-size: 18px;
		opacity: 0.6;
	}

	/* ── FULL / MUSIC, top right ── */
	.corner {
		position: fixed;
		top: 12px;
		right: 12px;
		display: flex;
		gap: 10px;
	}

	.word {
		background: transparent;
		border: none;
		padding: 0.3em 0.4em;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 12px;
		letter-spacing: 0.3em;
		opacity: 0.55;
		cursor: pointer;
		transition: opacity 200ms ease;
	}

	.word:hover {
		opacity: 1;
	}

	/* ── under the picture, as under the music ── */
	.controls {
		position: absolute;
		left: 50%;
		bottom: 18px;
		transform: translateX(-50%);
		width: min(560px, calc(100% - 64px));
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 8px;
	}

	.name {
		max-width: 100%;
		font-family: 'Kurale', serif;
		font-size: 20px;
		line-height: 1.3;
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}

	.progress-row {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 14px;
	}

	.clock {
		font-size: 15px;
		opacity: 0.6;
		font-variant-numeric: tabular-nums;
		flex-shrink: 0;
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

	.buttons {
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 1.6rem;
	}

	.arrow {
		background: transparent;
		border: none;
		padding: 0.3em 0.4em;
		font-size: 20px;
		line-height: 1;
		opacity: 0.6;
		cursor: pointer;
		transition: opacity 200ms ease;
	}

	.arrow:hover:not(:disabled) { opacity: 1; }
	.arrow:disabled { opacity: 0.18; cursor: default; }

	.play {
		background: transparent;
		border: none;
		padding: 0.3em 0.6em;
		font-family: 'Noto Sans JP', sans-serif;
		font-size: 15px;
		letter-spacing: 0.4em;
		cursor: pointer;
	}

	.play .ink-mark { padding-right: calc(0.45em - 0.4em); }

	/* ── full screen: the picture alone on black, the controls over its foot ── */
	.full {
		background: #000;
		--foot: 0px;
	}

	.full .room {
		inset: 0;
	}

	.full video {
		border-radius: 0;
	}

	.full .controls {
		left: 0;
		right: 0;
		bottom: 0;
		width: auto;
		transform: none;
		padding: 64px max(32px, calc(50% - 280px)) 22px;
		background: linear-gradient(to top, rgba(0, 0, 0, 0.62), rgba(0, 0, 0, 0));
		color: #e8e8e8;
		--line: #e8e8e8;
	}

	.full .corner {
		color: #e8e8e8;
	}

	.full .failed {
		color: #e8e8e8;
	}

	/* a small window (the mini player's size): the name gives way, the picture keeps its room */
	@media (max-height: 420px) {
		.video:not(.full) { --foot: 100px; }
		.video:not(.full) .name { display: none; }
		.room { top: 36px; left: 24px; right: 24px; }
	}

	.controls,
	.corner {
		transition: opacity 500ms ease;
	}

	.resting .controls,
	.resting .corner {
		opacity: 0;
		pointer-events: none;
	}
</style>
