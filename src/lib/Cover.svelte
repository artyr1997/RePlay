<script lang="ts">
	// Album art from the track's file (or a cover.jpg beside it); a quiet ink frame when there is none.
	import { coverUrl } from './music';

	let { trackId, title = '', size = 120 }: { trackId: number; title?: string; size?: number } = $props();

	let failed = $state(false);
	$effect(() => {
		trackId;
		failed = false;
	});
</script>

{#if failed}
	<div class="placeholder" style="width: {size}px; height: {size}px; font-size: {size * 0.36}px;">
		{title.trim().charAt(0).toUpperCase() || '♪'}
	</div>
{:else}
	<img
		src={coverUrl(trackId)}
		alt=""
		width={size}
		height={size}
		draggable="false"
		onerror={() => (failed = true)}
	/>
{/if}

<style>
	img {
		display: block;
		object-fit: cover;
		border-radius: 6px;
		box-shadow: 0 2px 14px rgba(0, 0, 0, 0.12);
		flex-shrink: 0;
	}

	.placeholder {
		display: flex;
		align-items: center;
		justify-content: center;
		flex-shrink: 0;
		border: 2px solid var(--line);
		border-radius: 6px;
		font-family: 'Kurale', serif;
		color: var(--text);
		opacity: 0.8;
	}
</style>
