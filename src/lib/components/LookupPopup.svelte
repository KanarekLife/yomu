<script lang="ts">
  import { fly, fade } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { glossary } from '../glossary';
  import type { LookupResult, Token } from '../types';
  import Pitch from './Pitch.svelte';
  import Icon from './Icon.svelte';
  import DictionarySetup from './DictionarySetup.svelte';

  let {
    result,
    token,
    anchor,
    loading,
    onclose,
    onsettings,
  }: {
    result: LookupResult | null;
    token: Token | null;
    anchor: DOMRect;
    loading: boolean;
    onclose: () => void;
    onsettings: () => void;
  } = $props();

  let vw = $state(window.innerWidth);
  let vh = $state(window.innerHeight);
  const sheet = $derived(vw < 680);
  const W = 420;
  const style = $derived.by(() => {
    if (sheet) return '';
    const left = Math.min(Math.max(12, anchor.left + anchor.width / 2 - W / 2), vw - W - 12);
    const below = anchor.bottom + 10;
    const spaceBelow = vh - below - 12;
    const spaceAbove = anchor.top - 22;
    if (spaceBelow >= 260 || spaceBelow >= spaceAbove)
      return `left:${left}px;top:${below}px;max-height:${Math.max(200, spaceBelow)}px`;
    return `left:${left}px;bottom:${vh - anchor.top + 10}px;max-height:${spaceAbove}px`;
  });
  const hasKanji = (s: string) => /[㐀-鿿\u{20000}-\u{2fa1f}々]/u.test(s);
</script>

<svelte:window bind:innerWidth={vw} bind:innerHeight={vh} />

{#if sheet}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="scrim" transition:fade={{ duration: 160 }} onclick={onclose}></div>
{/if}

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="popup card"
  class:sheet
  {style}
  onclick={(e) => e.stopPropagation()}
  transition:fly={sheet ? { y: 320, duration: 240, easing: cubicOut, opacity: 1 } : { y: 8, duration: 160, easing: cubicOut }}
>
  {#if sheet}<div class="grab"></div>{/if}
  <button class="icon-btn close" onclick={onclose} title="Close (Esc)"><Icon name="close" size={18} /></button>

  {#if token && !token.punct}
    <div class="tokinfo muted">
      <span class="jp">{token.surface}</span>
      {#if token.base && token.base !== token.surface}→ <span class="jp">{token.base}</span>{/if}
      {#if token.reading}· <span class="jp">{token.reading}</span>{/if}
      · {token.pos}
    </div>
  {/if}

  {#if loading && !result}
    <div class="skeleton"><div></div><div></div><div></div></div>
  {:else if result && !result.has_dictionaries}
    <div class="hint">
      <p class="muted">Install dictionaries to see definitions. One click, ~50 MB.</p>
      <DictionarySetup compact />
      <button class="link" onclick={onsettings}>More options</button>
    </div>
  {:else if result && result.entries.length === 0}
    <p class="muted hint">No entries found.</p>
  {:else if result}
    <div class="entries">
      {#each result.entries as e, i (i)}
        <section class="entry" in:fade={{ duration: 140, delay: Math.min(i, 6) * 25 }}>
          <div class="head">
            <span class="headword jp">
              {#if hasKanji(e.expression) && e.reading !== e.expression}
                <ruby>{e.expression}<rt>{e.reading}</rt></ruby>
              {:else}{e.expression}{/if}
            </span>
            {#each e.frequencies as f}
              <span class="chip freq" title={f.dictionary}>{f.dictionary.slice(0, 14)} <b>{f.value}</b></span>
            {/each}
          </div>
          {#if e.reasons.length}
            <div class="reasons muted">« {e.reasons.join(' ← ')}</div>
          {/if}
          {#if e.pitches.length}
            <div class="pitches">
              {#each e.pitches.slice(0, 1) as p}
                {#each p.positions as pos}<Pitch reading={p.reading} position={pos} />{/each}
              {/each}
            </div>
          {/if}
          {#each e.definitions as d}
            <div class="def">
              <div class="tags">
                {#each d.tags as t}<span class="chip tag {t.category}" title={t.notes}>{t.name}</span>{/each}
                <span class="dict">{d.dictionary}</span>
              </div>
              <div class="gloss" use:glossary={d.glossary}></div>
            </div>
          {/each}
        </section>
      {/each}
    </div>
  {/if}
</div>

<style>
  .scrim { position: fixed; inset: 0; background: #0004; z-index: 40; }
  .popup {
    position: fixed; z-index: 41; width: 420px; overflow-y: auto; padding: 14px 16px 16px;
    box-shadow: var(--shadow); font-size: 15px; overscroll-behavior: contain;
  }
  .popup.sheet {
    left: 0; right: 0; bottom: 0; width: auto; max-height: 62vh; border-radius: 18px 18px 0 0;
    padding-bottom: calc(16px + env(safe-area-inset-bottom)); border-bottom: none;
  }
  .grab { width: 40px; height: 4px; border-radius: 2px; background: var(--border); margin: -4px auto 10px; }
  .close { position: absolute; top: 6px; right: 6px; width: 34px; height: 34px; }
  .tokinfo { font-size: 13px; padding-right: 36px; margin-bottom: 8px; }
  .entries { display: flex; flex-direction: column; }
  .entry { padding: 10px 0; border-top: 1px solid var(--border); }
  .entry:first-child { border-top: none; padding-top: 0; }
  .head { display: flex; flex-wrap: wrap; align-items: baseline; gap: 6px 8px; padding-right: 30px; }
  .headword { font-size: 28px; line-height: 1.7; }
  .headword rt { font-size: 0.62em; color: var(--text); opacity: 0.8; padding-bottom: 3px; letter-spacing: 0.02em; }
  .reasons { font-size: 12px; margin-top: 2px; }
  .pitches { display: flex; gap: 12px; margin-top: 4px; }
  .chip { font-size: 11px; padding: 2px 7px; border-radius: 999px; background: var(--surface-2); white-space: nowrap; }
  .chip.freq { background: var(--accent-soft); }
  .chip.partOfSpeech { background: #3b82f622; }
  .chip.popular, .chip.frequent { background: #16a34a22; }
  .def { margin-top: 8px; }
  .tags { display: flex; flex-wrap: wrap; gap: 4px; align-items: center; margin-bottom: 4px; }
  .dict { font-size: 11px; color: var(--muted); margin-left: auto; }
  .gloss { line-height: 1.55; }
  .gloss :global(ol) { margin: 0; padding-left: 20px; }
  .gloss :global(ul) { margin: 2px 0; padding-left: 18px; }
  .gloss :global(table) { border-collapse: collapse; font-size: 13px; }
  .gloss :global(td), .gloss :global(th) { border: 1px solid var(--border); padding: 2px 6px; }
  .gloss :global(.sc-link) { color: var(--accent); }
  /* Jitendex / structured-content conventions */
  .gloss :global([data-sc-content='sense']) { margin: 2px 0 6px; }
  .gloss :global([data-sc-content='glossary']) { margin: 0; padding-left: 16px; }
  .gloss :global([data-sc-content='glossary'] li) { display: inline; }
  .gloss :global([data-sc-content='glossary'] li:not(:last-child))::after { content: '; '; }
  .gloss :global([data-sc-class='tag']) {
    display: inline-block; font-size: 11px; padding: 0 6px; margin: 0 4px 2px 0; border-radius: 999px;
    background: #3b82f622; vertical-align: 1px;
  }
  .gloss :global([data-sc-content='example-sentence']) {
    font-size: 13px; color: var(--muted); border-left: 2px solid var(--border); padding-left: 8px; margin: 4px 0 4px 16px;
  }
  .gloss :global([data-sc-content='example-keyword']) { color: var(--accent); }
  .gloss :global([data-sc-content='attribution']), .gloss :global([data-sc-content='attribution-footnote']),
  .gloss :global([data-sc-content='xref']), .gloss :global([data-sc-content='forms']) { font-size: 12px; color: var(--muted); }
  .gloss :global([data-sc-content='forms']) { display: none; }
  .hint { text-align: center; padding: 8px 0; }
  .hint p { margin: 4px 0 10px; }
  .link { all: unset; cursor: pointer; color: var(--accent); font-size: 13px; margin-top: 8px; display: inline-block; }
  .skeleton div { height: 14px; border-radius: 6px; margin: 10px 0; background: linear-gradient(90deg, var(--surface-2), var(--border), var(--surface-2)); background-size: 200% 100%; animation: sh 1.1s infinite linear; }
  .skeleton div:first-child { width: 40%; height: 24px; }
  .skeleton div:last-child { width: 70%; }
  @keyframes sh { to { background-position: -200% 0; } }
</style>
