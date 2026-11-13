<script lang="ts">
	// A video over the whole window: the picture in the middle, its name, the line and the controls
	// under it (they go away while it plays and the mouse rests). The file comes through Tauri's
	// asset protocol, so WebKit plays it itself: MP4 / M4V / MOV (H.264, HEVC), WebM.
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
	let restTimer: ReturnType<typeof setTimeout> | null = null;

	const path = $derived(paths[index]);
	const name = $derived((path.split('/').pop() ?? path).replace(/\.[^.]+$/, ''));
	const progress = $derived(duration > 0 ? time / duration : 0);

	// a new file: from its start
	$effect(() => {
		path;
		failed = false;
		time = 0;
		duration = 0;
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
		const bar = e.currentTarget as HTMLElement;
		const rect = bar.getBoundingClientRect();
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

	// the controls rest with the mouse while the video plays
	function wake() {
		resting = false;
		if (restTimer) clearTimeout(restTimer);
		restTimer = setTimeout(() => (resting = playing), 2500);
	}
	$effect(() => {
		if (!playing) resting = false;
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
			close();
		}
	}

	onMount(() => {
		// the window's own keys while the video is up (RePlay's are held, see App.svelte)
		window.addEventListener('keydown', keydown);
		return () => {
			window.removeEventListener('keydown', keydown);
			if (restTimer) clearTimeout(restTimer);
		};
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="video fade-in" class:resting onmousemove={wake}>
	{#key path}
		<!-- svelte-ignore a11y_media_has_caption -->
		<video
			bind:this={element}
			src={convertFileSrc(path)}
			autoplay
			bind:currentTime={time}
			bind:duration
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

	<button class="close" onclick={close} title="Back to the music (Esc)">MUSIC</button>

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
			<button class="full" onclick={toggleFullscreen} title="Full screen (F, or a double click)">FULL</button>
		</div>
	</div>
</div>

<style>
	.video {
		position: fixed;
		inset: 0;
		z-index: 50;
		background: var(--bg);
		cursor: default;
	}

	.video.resting {
		cursor: none;
	}

	video {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		object-fit: contain;
		/* the paper round the picture stays */
		background: transparent;
	}

	.failed {
		position: absolute;
		top: 45%;
		width: 100%;
		text-align: center;
		font-family: 'Kurale', serif;
		font-size: 18px;
		opacity: 0.6;
	}

	.close,
	.full {
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

	.close:hover,
	.full:hover {
		opacity: 1;
	}

	.close {
		position: fixed;
		top: 12px;
		right: 12px;
	}

	/* the paper under the controls, so they read over a bright picture too */
	.controls {
		position: fixed;
		left: 50%;
		bottom: 20px;
		transform: translateX(-50%);
		width: min(560px, calc(100% - 48px));
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 6px;
		padding: 12px 20px 10px;
		border-radius: 12px;
		background: color-mix(in srgb, var(--bg) 82%, transparent);
		backdrop-filter: blur(12px);
	}

	.controls,
	.close {
		transition: opacity 500ms ease;
	}

	.resting .controls,
	.resting .close {
		opacity: 0;
		pointer-events: none;
	}

	.name {
		max-width: 100%;
		font-family: 'Kurale', serif;
		font-size: 18px;
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
		position: relative;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 1.6rem;
		width: 100%;
	}

	.full {
		position: absolute;
		right: 0;
	}

	.arrow {
		background: transparent;
		border: none;
		padding: 0.3em 0.4em;
		font-size: 20px;
		line-height: 1;
		opacity: 0.6;
		cursor: pointer;
	}

	.arrow:disabled {
		opacity: 0.2;
		cursor: default;
	}

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
</style>
