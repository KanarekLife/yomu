<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { flip } from 'svelte/animate';
  import { scale } from 'svelte/transition';
  import { getCurrentWebview } from '@tauri-apps/api/webview';
  import { listen } from '@tauri-apps/api/event';
  import { api, errorMessage } from '../api';
  import { app } from '../state.svelte';
  import type { BookSummary } from '../types';
  import Icon from './Icon.svelte';
  import DictionarySetup from './DictionarySetup.svelte';

  let books = $state<BookSummary[]>([]);
  let covers = $state<Record<number, string | null>>({});
  let loading = $state(true);
  let importing = $state(false);
  let dragging = $state(false);
  let menuFor = $state<number | null>(null);
  let unlisten: (() => void) | undefined;
  let unlistenLib: (() => void) | undefined;

  async function refresh() {
    try {
      books = await api.listBooks();
      for (const b of books) {
        if (b.has_cover && !(b.id in covers)) {
          covers[b.id] = null;
          api.bookCover(b.id).then((u) => (covers[b.id] = u)).catch(() => {});
        }
      }
    } catch (e) {
      app.toast(errorMessage(e), 'error');
    } finally {
      loading = false;
    }
  }

  async function importBook() {
    importing = true;
    try {
      const b = await api.importBook();
      if (b) {
        app.toast(`Imported “${b.title}”`);
        await refresh();
        app.requestSync();
      }
    } catch (e) {
      app.toast(errorMessage(e), 'error');
    } finally {
      importing = false;
    }
  }

  async function importPaths(paths: string[]) {
    importing = true;
    for (const p of paths.filter((p) => p.toLowerCase().endsWith('.epub'))) {
      try {
        const b = await api.importBookPath(p);
        app.toast(`Imported “${b.title}”`);
      } catch (e) {
        app.toast(errorMessage(e), 'error');
      }
    }
    importing = false;
    refresh();
    app.requestSync();
  }

  async function remove(b: BookSummary) {
    menuFor = null;
    if (!confirm(`Remove “${b.title}” from the library?`)) return;
    await api.deleteBook(b.id).catch((e) => app.toast(errorMessage(e), 'error'));
    books = books.filter((x) => x.id !== b.id);
    app.requestSync();
  }

  // ---- sync: pull down to sync (touch), or the header button -------------------
  const PULL_AT = 64;
  let pull = $state(0);
  let pullStart: number | null = null;
  const syncOn = $derived(!!app.sync && app.sync.mode !== 'off' && (app.sync.mode !== 'server' || app.sync.signed_in));

  async function syncNow() {
    await app.runSync(true);
    refresh();
  }
  function touchStart(e: TouchEvent) {
    const el = e.currentTarget as HTMLElement;
    pullStart = syncOn && !app.syncing && el.scrollTop <= 0 ? e.touches[0].clientY : null;
  }
  function touchMove(e: TouchEvent) {
    if (pullStart == null) return;
    const dy = e.touches[0].clientY - pullStart;
    pull = dy > 0 ? Math.min(PULL_AT * 1.5, dy * 0.5) : 0;
  }
  function touchEnd() {
    if (pull >= PULL_AT) syncNow();
    pull = 0;
    pullStart = null;
  }

  const pct = (b: BookSummary) =>
    b.sentence_count > 1 ? Math.round((b.position / (b.sentence_count - 1)) * 100) : 0;

  onMount(async () => {
    refresh();
    listen('library-changed', () => refresh()).then((u) => (unlistenLib = u));
    try {
      unlisten = await getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === 'over' || e.payload.type === 'enter') dragging = true;
        else if (e.payload.type === 'drop') {
          dragging = false;
          importPaths(e.payload.paths);
        } else dragging = false;
      });
    } catch {
      /* not available on mobile */
    }
  });
  onDestroy(() => {
    unlisten?.();
    unlistenLib?.();
    Object.values(covers).forEach((u) => u && URL.revokeObjectURL(u));
  });
</script>

<svelte:window onclick={() => (menuFor = null)} />

<header class="top">
  <h1>Library</h1>
  <div class="actions">
    {#if syncOn}
      <button class="icon-btn" class:spin={app.syncing} title="Sync now" disabled={app.syncing} onclick={syncNow}>
        <Icon name="refresh" />
      </button>
    {/if}
    <button class="icon-btn" title="Settings" onclick={() => app.go({ name: 'settings', from: { name: 'library' } })}>
      <Icon name="settings" />
    </button>
    <button class="btn primary" onclick={importBook} disabled={importing}>
      <Icon name="plus" size={18} />{importing ? 'Importing…' : 'Import EPUB'}
    </button>
  </div>
</header>

<!-- svelte-ignore a11y_no_static_element_interactions (pull to sync; the header button is the accessible route) -->
<section class="scroll" class:dragging ontouchstart={touchStart} ontouchmove={touchMove} ontouchend={touchEnd} ontouchcancel={touchEnd}>
  {#if pull > 0 || app.syncing}
    <div class="pull" style="height:{app.syncing && pull === 0 ? 40 : pull}px" class:ready={pull >= PULL_AT}>
      <span class:spin={app.syncing} style="transform:rotate({pull * 4}deg)"><Icon name="refresh" size={20} /></span>
    </div>
  {/if}
  {#if !loading && books.length === 0}
    <div class="empty" in:scale={{ start: 0.96, duration: 220 }}>
      <div class="glyph jp">読</div>
      <h2>Your shelf is empty</h2>
      <p class="muted">Import a Japanese light novel in EPUB format to start reading, one sentence at a time.</p>
      <button class="btn primary" onclick={importBook}><Icon name="plus" size={18} />Import EPUB</button>
      <div class="setup-wrap"><DictionarySetup /></div>
    </div>
  {:else}
    {#if !loading && app.dicts.length === 0}<div class="banner"><DictionarySetup /></div>{/if}
    <div class="grid">
      {#each books as b (b.id)}
        <article class="book" animate:flip={{ duration: 220 }} in:scale={{ start: 0.94, duration: 200 }}>
          <button class="cover-btn" onclick={() => app.go({ name: 'reader', bookId: b.id })} title={b.title}>
            <div class="cover">
              {#if covers[b.id]}
                <img src={covers[b.id]} alt="" loading="lazy" />
              {:else}
                <div class="placeholder jp">{b.title.slice(0, 12)}</div>
              {/if}
              <div class="progress"><div style="width:{pct(b)}%"></div></div>
            </div>
          </button>
          <div class="meta">
            <div class="text">
              <div class="title jp" title={b.title}>{b.title}</div>
              <div class="sub muted">{b.author || '—'} · {pct(b)}%</div>
            </div>
            <div class="menu-wrap">
              <button class="icon-btn small" title="More" onclick={(e) => { e.stopPropagation(); menuFor = menuFor === b.id ? null : b.id; }}>
                <Icon name="more" size={18} />
              </button>
              {#if menuFor === b.id}
                <div class="menu card" in:scale={{ start: 0.9, duration: 120 }}>
                  <button class="danger" onclick={() => remove(b)}><Icon name="trash" size={16} />Remove</button>
                </div>
              {/if}
            </div>
          </div>
        </article>
      {/each}
    </div>
  {/if}
  {#if dragging}
    <div class="drop">Drop EPUB files to import</div>
  {/if}
</section>

<style>
  .top {
    display: flex; align-items: center; justify-content: space-between; gap: 12px;
    padding: calc(14px + env(safe-area-inset-top)) 20px 10px;
  }
  h1 { font-size: 26px; margin: 0; letter-spacing: -0.02em; }
  .actions { display: flex; gap: 6px; align-items: center; }
  .scroll { flex: 1; overflow-y: auto; overscroll-behavior-y: contain; padding: 8px 20px calc(32px + env(safe-area-inset-bottom)); position: relative; }
  .pull { display: flex; align-items: center; justify-content: center; overflow: hidden; color: var(--muted); opacity: 0.6; }
  .pull.ready { opacity: 1; color: var(--accent); }
  .pull span { display: inline-flex; }
  .spin, .pull span.spin { animation: spin 0.9s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(140px, 1fr)); gap: 22px 18px; }
  @media (min-width: 900px) { .grid { grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); } }
  .book { min-width: 0; }
  .cover-btn { all: unset; display: block; width: 100%; cursor: pointer; border-radius: 10px; }
  .cover {
    position: relative; aspect-ratio: 2 / 3; border-radius: 10px; overflow: hidden; background: var(--surface-2);
    box-shadow: var(--shadow); transition: transform 200ms var(--ease), box-shadow 200ms var(--ease);
  }
  .cover-btn:hover .cover { transform: translateY(-3px); }
  .cover-btn:active .cover { transform: scale(0.98); }
  .cover img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .placeholder {
    height: 100%; display: grid; place-items: center; padding: 16px; text-align: center; font-size: 18px;
    writing-mode: vertical-rl; background: linear-gradient(160deg, var(--surface), var(--surface-2));
  }
  .progress { position: absolute; left: 0; right: 0; bottom: 0; height: 4px; background: #0003; }
  .progress div { height: 100%; background: var(--accent); }
  .meta { display: flex; align-items: flex-start; gap: 4px; margin-top: 10px; }
  .text { flex: 1; min-width: 0; }
  .title { font-weight: 600; font-size: 14px; line-height: 1.35; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .sub { font-size: 12px; margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .icon-btn.small { width: 30px; height: 30px; border-radius: 8px; }
  .menu-wrap { position: relative; }
  .menu { position: absolute; right: 0; top: 32px; z-index: 5; padding: 4px; box-shadow: var(--shadow); transform-origin: top right; }
  .menu button { all: unset; display: flex; gap: 8px; align-items: center; padding: 8px 12px; border-radius: 8px; cursor: pointer; white-space: nowrap; font-size: 14px; }
  .menu button:hover { background: var(--surface-2); }
  .danger { color: #dc2626; }
  .empty { max-width: 420px; margin: 10vh auto 0; text-align: center; display: flex; flex-direction: column; align-items: center; gap: 10px; }
  .glyph { font-size: 64px; color: var(--accent); line-height: 1; }
  .empty h2 { margin: 6px 0 0; }
  .banner { max-width: 520px; margin: 0 auto 20px; }
  .setup-wrap { width: 100%; margin-top: 22px; }
  .drop {
    position: absolute; inset: 12px; border: 2px dashed var(--accent); border-radius: 16px; display: grid; place-items: center;
    background: var(--accent-soft); font-weight: 600; pointer-events: none;
  }
</style>
