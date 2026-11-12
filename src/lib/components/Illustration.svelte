<script lang="ts">
  import { onDestroy } from 'svelte';
  import { fade } from 'svelte/transition';
  import { api, errorMessage } from '../api';

  let { bookId, path }: { bookId: number; path: string } = $props();

  let url = $state<string | null>(null);
  let error = $state<string | null>(null);
  let zoom = $state(false);

  // Object URLs are shared between steps showing the same picture and freed with the reader.
  $effect(() => {
    const p = path;
    error = null;
    imageUrl(bookId, p).then(
      (u) => { if (p === path) url = u; },
      (e) => { if (p === path) error = errorMessage(e); },
    );
  });
  onDestroy(() => (zoom = false));

  /** Mount at <body>: the reader's slide animation transforms our ancestors,
   *  which would trap a `position: fixed` overlay inside the slot. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return { destroy: () => node.remove() };
  }
</script>

<script lang="ts" module>
  const urls = new Map<string, Promise<string>>();
  export function imageUrl(bookId: number, path: string): Promise<string> {
    const k = `${bookId}|${path}`;
    let u = urls.get(k);
    if (!u) {
      u = api.bookImage(bookId, path);
      u.catch(() => urls.delete(k));
      urls.set(k, u);
    }
    return u;
  }
  /** Free every cached picture (when leaving the reader). */
  export function releaseImages() {
    for (const u of urls.values()) u.then(URL.revokeObjectURL, () => {});
    urls.clear();
  }
</script>

<figure class="ill">
  {#if url}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <img src={url} alt="Illustration" draggable="false" onclick={(e) => { e.stopPropagation(); zoom = true; }} />
  {:else if error}
    <figcaption class="muted">Illustration unavailable: {error}</figcaption>
  {:else}
    <div class="ph"></div>
  {/if}
</figure>

{#if zoom && url}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="zoom" use:portal transition:fade={{ duration: 150 }} onclick={(e) => { e.stopPropagation(); zoom = false; }}>
    <img src={url} alt="Illustration" />
  </div>
{/if}

<style>
  .ill { margin: 0; display: flex; justify-content: center; }
  .ill img {
    display: block; max-width: 100%; max-height: calc(100dvh - 210px); object-fit: contain;
    border-radius: 6px; box-shadow: var(--shadow); cursor: zoom-in; user-select: none;
  }
  .ph { width: min(60vw, 320px); aspect-ratio: 2 / 3; border-radius: 6px; background: var(--surface-2); }
  figcaption { font-family: var(--font-ui); font-size: 14px; text-align: center; }
  .zoom {
    position: fixed; inset: 0; z-index: 40; background: #000e; display: flex; align-items: center; justify-content: center;
    cursor: zoom-out; padding: env(safe-area-inset-top) 0 env(safe-area-inset-bottom);
  }
  .zoom img { max-width: 100vw; max-height: 100dvh; object-fit: contain; }
</style>
