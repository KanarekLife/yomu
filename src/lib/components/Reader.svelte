<script lang="ts">
  import { onDestroy, onMount, untrack } from 'svelte';
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { SvelteMap } from 'svelte/reactivity';
  import { api, errorMessage } from '../api';
  import { app } from '../state.svelte';
  import type { BookDetail, LookupResult, SentenceView, Token, Translation } from '../types';
  import Icon from './Icon.svelte';
  import Sentence from './Sentence.svelte';
  import LookupPopup from './LookupPopup.svelte';
  import TranslationPanel from './TranslationPanel.svelte';
  import ReaderOptions from './ReaderOptions.svelte';

  let { bookId }: { bookId: number } = $props();

  const CHUNK = 32;
  let detail = $state<BookDetail | null>(null);
  let pos = $state(0);
  let dir = $state(1);
  const cache = new SvelteMap<number, SentenceView>();
  const inflight = new Set<number>();
  const total = $derived(detail?.sentence_count ?? 0);
  const current = $derived(cache.get(pos));
  const prev = $derived(cache.get(pos - 1));
  const chapter = $derived(detail?.chapters.findLast((c) => c.start <= pos) ?? null);
  const chapterLabel = $derived.by(() => {
    if (!chapter || !detail) return '';
    if (chapter.level === 0) return chapter.title === detail.title ? '' : chapter.title;
    const parent = detail.chapters.findLast((c) => c.level === 0 && c.start <= chapter.start);
    return parent && parent.title !== detail.title ? `${parent.title} · ${chapter.title}` : chapter.title;
  });
  const progress = $derived(total > 1 ? pos / (total - 1) : 0);

  // ---- loading -------------------------------------------------------------
  async function loadChunk(start: number) {
    if (start < 0 || start >= total || inflight.has(start) || cache.has(start)) return;
    inflight.add(start);
    try {
      const list = await api.sentences(bookId, start, CHUNK);
      for (const s of list) cache.set(s.idx, s);
    } catch (e) {
      app.toast(errorMessage(e), 'error');
    } finally {
      inflight.delete(start);
    }
  }
  function ensure(p: number) {
    const c = Math.floor(p / CHUNK) * CHUNK;
    loadChunk(c);
    if (p - c < 8) loadChunk(c - CHUNK);
    if (c + CHUNK - p <= 8) loadChunk(c + CHUNK);
  }
  $effect(() => {
    if (detail) ensure(pos);
  });

  // ---- navigation ----------------------------------------------------------
  let saveTimer: ReturnType<typeof setTimeout> | undefined;
  function persist() {
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => api.setPosition(bookId, pos).catch(() => {}), 400);
  }
  function jump(target: number) {
    if (!total) return;
    const t = Math.max(0, Math.min(total - 1, target));
    if (t === pos) return;
    dir = t > pos ? 1 : -1;
    pos = t;
    closeLookup();
    soft = null;
    persist();
  }
  const next = () => jump(pos + 1);
  const back = () => jump(pos - 1);

  function slide(node: Element, { d, out = false }: { d: number; out?: boolean }) {
    return {
      duration: out ? 140 : 200,
      easing: cubicOut,
      css: (t: number) => `transform: translateX(${(1 - t) * 28 * (out ? -d : d)}px); opacity: ${t}`,
    };
  }

  // ---- lookup --------------------------------------------------------------
  let lookup = $state<{ result: LookupResult | null; token: Token | null; anchor: DOMRect; loading: boolean } | null>(null);
  let highlight = $state<{ start: number; len: number } | null>(null);
  let soft = $state<{ start: number; len: number } | null>(null);
  let lookupSeq = 0;

  async function pick(offset: number, el: HTMLElement) {
    if (swiped || !current) return;
    const s = current;
    const token = s.tokens.find((t) => offset >= t.start && offset < t.end) ?? null;
    if (!token || token.punct) {
      closeLookup();
      return;
    }
    lastPick = { offset, el };
    const seq = ++lookupSeq;
    highlight = { start: offset, len: token.end - offset };
    lookup = { result: null, token, anchor: el.getBoundingClientRect(), loading: true };
    try {
      const result = await api.lookup(s.text, offset);
      if (seq !== lookupSeq || !lookup) return;
      lookup = { ...lookup, result, loading: false };
      if (result.length > 0) highlight = { start: offset, len: result.length };
    } catch (e) {
      if (seq === lookupSeq) closeLookup();
      app.toast(errorMessage(e), 'error');
    }
  }
  let lastPick: { offset: number; el: HTMLElement } | null = null;
  $effect(() => {
    const n = app.dicts.length;
    untrack(() => {
      if (n && lookup?.result && !lookup.result.has_dictionaries && lastPick) pick(lastPick.offset, lastPick.el);
    });
  });
  function closeLookup() {
    lookupSeq++;
    lookup = null;
    highlight = null;
  }

  // ---- translation ---------------------------------------------------------
  type TState = { status: 'loading' | 'done' | 'error'; t?: Translation; err?: string };
  const translations = new SvelteMap<number, TState>();
  let trTimer: ReturnType<typeof setTimeout> | undefined;

  async function translate(idx: number, force = false): Promise<void> {
    const existing = translations.get(idx);
    if (!force && existing && existing.status !== 'error') return;
    translations.set(idx, { status: 'loading' });
    try {
      const t = await api.translate(bookId, idx, force);
      translations.set(idx, { status: 'done', t });
    } catch (e) {
      translations.set(idx, { status: 'error', err: errorMessage(e) });
    }
  }
  $effect(() => {
    const p = pos;
    if (!app.settings.translation || !detail) return;
    untrack(() => scheduleTranslation(p));
  });
  function scheduleTranslation(p: number) {
    clearTimeout(trTimer);
    const have = translations.get(p);
    if (have && have.status !== 'error') {
      if (app.settings.prefetchTranslation && p + 1 < total) trTimer = setTimeout(() => translate(p + 1), 600);
      return;
    }
    translations.set(p, { status: 'loading' });
    // Instant if cached; otherwise wait briefly so fast paging does not
    // trigger paid requests for every skipped sentence.
    api.cachedTranslation(bookId, p).then((c) => {
      if (c) translations.set(p, { status: 'done', t: c });
      else translations.delete(p);
      trTimer = setTimeout(async () => {
        if (pos !== p) return;
        await translate(p);
        if (app.settings.prefetchTranslation && pos === p && p + 1 < total) translate(p + 1);
      }, c ? 0 : 280);
    });
  }

  function hoverPart(text: string | null) {
    if (!text || !current) return (soft = null);
    const i = current.text.indexOf(text);
    soft = i < 0 ? null : { start: Array.from(current.text.slice(0, i)).length, len: Array.from(text).length };
  }

  // ---- toggles ---------------------------------------------------------------
  function toggleFurigana() {
    app.settings.furigana = !app.settings.furigana;
    app.save();
  }
  function toggleTranslation() {
    app.settings.translation = !app.settings.translation;
    app.save();
  }

  // ---- input ---------------------------------------------------------------
  let tocOpen = $state(false);
  let optionsOpen = $state(false);
  let swiped = false;
  let sx = 0, sy = 0, tracking = false;
  function down(e: PointerEvent) {
    if (e.pointerType === 'mouse') return;
    tracking = true;
    sx = e.clientX;
    sy = e.clientY;
  }
  function up(e: PointerEvent) {
    if (!tracking) return;
    tracking = false;
    const dx = e.clientX - sx, dy = e.clientY - sy;
    if (Math.abs(dx) > 60 && Math.abs(dy) < Math.abs(dx) * 0.6) {
      swiped = true;
      setTimeout(() => (swiped = false), 50);
      if (dx < 0) next();
      else back();
    }
  }
  function key(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    switch (e.key) {
      case 'ArrowRight': case 'ArrowDown': case 'PageDown': case ' ': case 'j': case 'l':
        e.preventDefault(); next(); break;
      case 'ArrowLeft': case 'ArrowUp': case 'PageUp': case 'k': case 'h':
        e.preventDefault(); back(); break;
      case 'Home': jump(0); break;
      case 'End': jump(total - 1); break;
      case 'f': toggleFurigana(); break;
      case 't': toggleTranslation(); break;
      case 'Escape':
        if (lookup) closeLookup();
        else if (optionsOpen) optionsOpen = false;
        else if (tocOpen) tocOpen = false;
        else exit();
        break;
    }
  }

  // ---- scrubbing -----------------------------------------------------------
  let scrub = $state<number | null>(null);
  const scrubChapter = $derived(scrub == null ? null : detail?.chapters.findLast((c) => c.start <= scrub!) ?? null);

  async function reimport() {
    try {
      const b = await api.importBook();
      if (!b) return;
      tocOpen = false;
      cache.clear();
      if (b.id === bookId) {
        detail = await api.bookDetail(bookId);
        pos = Math.min(detail.position, Math.max(0, detail.sentence_count - 1));
        app.toast('Chapters updated');
      } else {
        app.go({ name: 'reader', bookId: b.id });
      }
    } catch (e) {
      app.toast(errorMessage(e), 'error');
    }
  }

  function exit() {
    clearTimeout(saveTimer);
    api.setPosition(bookId, pos).finally(() => app.go({ name: 'library' }));
  }

  onMount(async () => {
    try {
      detail = await api.bookDetail(bookId);
      pos = Math.min(detail.position, Math.max(0, detail.sentence_count - 1));
    } catch (e) {
      app.toast(errorMessage(e), 'error');
      app.go({ name: 'library' });
    }
  });
  onDestroy(() => {
    clearTimeout(saveTimer);
    clearTimeout(trTimer);
    if (detail) api.setPosition(bookId, pos).catch(() => {});
  });
</script>

<svelte:window onkeydown={key} />

<header class="bar top">
  <button class="icon-btn" title="Library (Esc)" onclick={exit}><Icon name="back" /></button>
  <div class="title">
    <div class="book jp">{detail?.title ?? ''}</div>
    {#if chapterLabel}<div class="chapter muted jp">{chapterLabel}</div>{/if}
  </div>
  <button class="icon-btn" title="Contents" onclick={() => (tocOpen = true)}><Icon name="toc" /></button>
  <button class="icon-btn" title="Settings" onclick={() => app.go({ name: 'settings', from: { name: 'reader', bookId } })}>
    <Icon name="settings" />
  </button>
</header>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="content" onpointerdown={down} onpointerup={up} onpointercancel={() => (tracking = false)} onclick={closeLookup}>
  <div class="inner" style="--fs:{app.settings.fontSize}px">
    {#if app.settings.showContext && prev && current}
      {#key pos}
        <div class="context jp" in:fade={{ duration: 200 }}>{prev.text}</div>
      {/key}
    {/if}
    <div class="stage">
      {#if current}
        {#key pos}
          <div class="slot" in:slide={{ d: dir }} out:slide={{ d: dir, out: true }}>
            <Sentence sentence={current} furigana={app.settings.furigana} {highlight} {soft} onpick={pick} />
          </div>
        {/key}
      {:else if detail}
        <div class="slot loading muted" in:fade={{ duration: 300, delay: 150 }}>…</div>
      {/if}
    </div>
    {#if app.settings.translation && current}
      <TranslationPanel
        state={translations.get(pos)}
        bind:showBreakdown={app.settings.showBreakdown}
        onretry={() => translate(pos, true)}
        onhover={hoverPart}
      />
    {/if}
  </div>
</div>

<footer class="bottom">
  <div class="progress">
    <div class="track">
      <div class="fill" style="transform:scaleX({scrub != null ? scrub / Math.max(1, total - 1) : progress})"></div>
    </div>
    <input
      type="range"
      min="0"
      max={Math.max(0, total - 1)}
      value={scrub ?? pos}
      aria-label="Position"
      oninput={(e) => (scrub = +e.currentTarget.value)}
      onchange={(e) => { jump(+e.currentTarget.value); scrub = null; }}
      onpointerup={() => setTimeout(() => (scrub = null), 0)}
    />
    {#if scrub != null}
      <div class="bubble card" style="left:{(scrub / Math.max(1, total - 1)) * 100}%" transition:fade={{ duration: 100 }}>
        {#if scrubChapter}<span class="jp">{scrubChapter.title}</span> · {/if}{scrub + 1}
      </div>
    {/if}
  </div>
  <div class="controls">
    <button class="icon-btn nav" title="Previous (←)" disabled={pos <= 0} onclick={back}><Icon name="prev" /></button>
    <div class="quick">
      <button class="pill" class:on={app.settings.furigana} onclick={toggleFurigana} title="Furigana (F)" aria-pressed={app.settings.furigana}>
        <span class="jp g">あ</span><span class="lbl">Furigana</span>
      </button>
      <button class="pill" class:on={app.settings.translation} onclick={toggleTranslation} title="Translation (T)" aria-pressed={app.settings.translation}>
        <span class="jp g">訳</span><span class="lbl">Translate</span>
      </button>
      <button class="pill" class:on={optionsOpen} onclick={() => (optionsOpen = !optionsOpen)} title="Reading options">
        <span class="g aa">Aa</span>
      </button>
    </div>
    <div class="count muted">{total ? pos + 1 : 0}/{total}<br /><span>{(progress * 100).toFixed(1)}%</span></div>
    <button class="icon-btn nav" title="Next (→)" disabled={pos >= total - 1} onclick={next}><Icon name="next" /></button>
  </div>
</footer>

{#if optionsOpen}
  <ReaderOptions onclose={() => (optionsOpen = false)} />
{/if}

{#if lookup}
  <LookupPopup
    result={lookup.result}
    token={lookup.token}
    anchor={lookup.anchor}
    loading={lookup.loading}
    onclose={closeLookup}
    onsettings={() => app.go({ name: 'settings', from: { name: 'reader', bookId } })}
  />
{/if}

{#if tocOpen && detail}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" transition:fade={{ duration: 160 }} onclick={() => (tocOpen = false)}></div>
  <nav class="toc card" transition:fly={{ x: -320, duration: 240, easing: cubicOut, opacity: 1 }}>
    <div class="toc-head">
      <h3>Contents</h3>
      <button class="icon-btn" onclick={() => (tocOpen = false)}><Icon name="close" size={18} /></button>
    </div>
    {#if detail.outdated}
      <div class="outdated">
        <p>This book was imported by an older version, so in-text chapters aren't detected yet.</p>
        <button class="btn" onclick={reimport}>Import the EPUB again</button>
      </div>
    {/if}
    <ul>
      {#each detail.chapters as c}
        <li>
          <button class:active={chapter?.idx === c.idx} class:sub={c.level > 0} onclick={() => { jump(c.start); tocOpen = false; }}>
            <span class="jp">{c.title}</span>
            <span class="muted">{Math.round((c.start / Math.max(1, total - 1)) * 100)}%</span>
          </button>
        </li>
      {/each}
    </ul>
  </nav>
{/if}

<style>
  .bar { display: flex; align-items: center; gap: 2px; padding: 6px 8px; }
  .top { padding-top: calc(6px + env(safe-area-inset-top)); }
  .title { flex: 1; min-width: 0; padding: 0 8px; }
  .book { font-size: 14px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .chapter { font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .content {
    flex: 1; overflow-y: auto; display: flex; flex-direction: column; justify-content: safe center;
    padding: 12px 20px; touch-action: pan-y;
  }
  .inner {
    width: 100%; max-width: 860px; margin: 0 auto; display: flex; flex-direction: column; gap: 22px;
    font-family: var(--font-jp); font-size: var(--fs);
  }
  .context { font-size: 0.5em; color: var(--muted); opacity: 0.7; text-align: center; line-height: 1.6; }
  .stage { display: grid; min-height: 3em; align-items: center; }
  .slot { grid-area: 1 / 1; will-change: transform, opacity; }
  .loading { text-align: center; }
  .inner :global(.panel) { font-family: var(--font-ui); }

  .bottom { padding: 4px 10px calc(8px + env(safe-area-inset-bottom)); display: flex; flex-direction: column; gap: 4px; }
  .progress { position: relative; padding: 8px 6px 4px; }
  .track { height: 4px; border-radius: 2px; background: var(--surface-2); overflow: hidden; }
  .fill { height: 100%; background: var(--accent); transform-origin: left; transition: transform 200ms var(--ease); }
  input[type='range'] {
    position: absolute; left: 0; right: 0; top: -4px; height: 28px; margin: 0; opacity: 0; cursor: pointer; width: 100%;
  }
  .bubble {
    position: absolute; bottom: 26px; transform: translateX(-50%); padding: 6px 10px; font-size: 12px; white-space: nowrap;
    box-shadow: var(--shadow); pointer-events: none; max-width: 70vw; overflow: hidden; text-overflow: ellipsis;
  }
  .controls { display: flex; align-items: center; gap: 6px; }
  .nav { width: 52px; height: 48px; }
  .quick { flex: 1; display: flex; justify-content: center; gap: 8px; }
  .pill {
    display: inline-flex; align-items: center; gap: 6px; height: 40px; padding: 0 14px; border-radius: 999px;
    border: 1px solid var(--border); background: var(--surface); color: var(--muted); font-size: 14px;
    transition: background 160ms var(--ease), color 160ms, border-color 160ms, transform 120ms var(--ease);
  }
  .pill:active { transform: scale(0.95); }
  .pill.on { background: var(--accent-soft); border-color: transparent; color: var(--accent); }
  .pill .g { font-size: 17px; font-weight: 600; }
  .pill .aa { font-family: var(--font-serif); }
  .count { font-size: 11px; text-align: right; font-variant-numeric: tabular-nums; line-height: 1.3; min-width: 64px; }
  @media (max-width: 520px) {
    .pill .lbl { display: none; }
    .pill { padding: 0 12px; }
    .count { min-width: 52px; }
  }

  .scrim { position: fixed; inset: 0; background: #0004; z-index: 30; }
  .toc {
    position: fixed; top: 0; bottom: 0; left: 0; width: min(340px, 86vw); z-index: 31; border-radius: 0 18px 18px 0;
    display: flex; flex-direction: column; padding-top: env(safe-area-inset-top); box-shadow: var(--shadow);
  }
  .toc-head { display: flex; align-items: center; justify-content: space-between; padding: 10px 8px 6px 18px; }
  .toc h3 { margin: 0; }
  .toc ul { list-style: none; margin: 0; padding: 0 8px 16px; overflow-y: auto; }
  .toc li button {
    all: unset; box-sizing: border-box; width: 100%; display: flex; justify-content: space-between; gap: 10px;
    padding: 11px 12px; border-radius: 10px; cursor: pointer; font-size: 15px;
  }
  .toc li button:hover { background: var(--surface-2); }
  .toc li button.active { background: var(--accent-soft); color: var(--accent); }
  .outdated { margin: 0 12px 10px; padding: 12px; border-radius: 12px; background: var(--accent-soft); font-size: 13px; }
  .outdated p { margin: 0 0 8px; }
  .toc li button.sub { padding-left: 30px; font-size: 14px; }
</style>
