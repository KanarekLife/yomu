<script lang="ts">
  import { slide, fade } from 'svelte/transition';
  import type { Translation } from '../types';
  import Icon from './Icon.svelte';

  let {
    state,
    showBreakdown = $bindable(true),
    onretry,
    onhover,
  }: {
    state: { status: 'loading' | 'done' | 'error'; t?: Translation; err?: string } | undefined;
    showBreakdown?: boolean;
    onretry: () => void;
    onhover: (text: string | null) => void;
  } = $props();
</script>

<div class="panel" transition:slide={{ duration: 200 }}>
  {#if !state || state.status === 'loading'}
    <div class="skeleton" in:fade={{ duration: 120 }}><div></div><div></div></div>
  {:else if state.status === 'error'}
    <div class="error">
      <span>{state.err}</span>
      <button class="btn" onclick={onretry}><Icon name="refresh" size={16} />Retry</button>
    </div>
  {:else if state.t}
    {@const t = state.t}
    <div in:fade={{ duration: 160 }}>
      <p class="tr">{t.translation}</p>
      {#if t.literal && t.literal !== t.translation}
        <p class="literal muted">Literal: {t.literal}</p>
      {/if}

      {#if t.breakdown.length || t.grammar.length || t.notes}
        <button class="toggle" onclick={() => (showBreakdown = !showBreakdown)}>
          <Icon name={showBreakdown ? 'up' : 'down'} size={16} />
          {showBreakdown ? 'Hide' : 'Show'} breakdown
        </button>
      {/if}

      {#if showBreakdown}
        <div transition:slide={{ duration: 180 }}>
          {#if t.breakdown.length}
            <ul class="parts">
              {#each t.breakdown as p}
                <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
                <li onmouseenter={() => onhover(p.text)} onmouseleave={() => onhover(null)}>
                  <span class="jp chunk">{p.text}</span>
                  {#if p.reading && p.reading !== p.text}<span class="reading jp">{p.reading}</span>{/if}
                  <span class="meaning">{p.meaning}</span>
                  {#if p.role}<span class="role">{p.role}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if t.grammar.length}
            <h4>Grammar</h4>
            <ul class="grammar">
              {#each t.grammar as g}
                <li><span class="jp pattern">{g.pattern}</span> {g.explanation}</li>
              {/each}
            </ul>
          {/if}
          {#if t.notes}
            <h4>Notes</h4>
            <p class="notes">{t.notes}</p>
          {/if}
        </div>
      {/if}
      <div class="foot muted">
        <span>{t.model}{t.cached ? ' · cached' : ''}</span>
        <button class="icon-btn mini" title="Translate again" onclick={onretry}><Icon name="refresh" size={14} /></button>
      </div>
    </div>
  {/if}
</div>

<style>
  .panel { width: 100%; max-width: 760px; margin: 0 auto; border-top: 1px solid var(--border); padding-top: 14px; font-size: 15px; }
  .tr { font-size: 18px; line-height: 1.5; margin: 0 0 6px; }
  .literal { font-size: 14px; margin: 0 0 8px; font-style: italic; }
  .toggle { all: unset; cursor: pointer; display: inline-flex; gap: 4px; align-items: center; color: var(--accent); font-size: 13px; padding: 4px 0; }
  /* One shared grid (rows use subgrid) so every meaning starts in the same column. */
  .parts { list-style: none; padding: 0; margin: 8px 0; display: grid; grid-template-columns: minmax(4em, max-content) 1fr; row-gap: 2px; }
  .parts li { display: grid; grid-column: 1 / -1; grid-template-columns: subgrid; column-gap: 14px; padding: 6px 8px; border-radius: 8px; transition: background 120ms; }
  .parts li:hover { background: var(--surface-2); }
  .chunk { font-size: 17px; grid-column: 1; grid-row: 1 / span 2; align-self: center; }
  .meaning, .role { grid-column: 2; }
  .reading { font-size: 11px; color: var(--muted); grid-column: 1; grid-row: 3; }
  .meaning { font-weight: 500; }
  .role { font-size: 12px; color: var(--muted); }
  h4 { margin: 12px 0 4px; font-size: 12px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }
  .grammar { padding-left: 18px; margin: 0; line-height: 1.5; }
  .pattern { font-weight: 600; color: var(--accent); margin-right: 4px; }
  .notes { margin: 0; line-height: 1.5; }
  .foot { display: flex; align-items: center; justify-content: flex-end; gap: 4px; font-size: 11px; margin-top: 6px; }
  .mini { width: 28px; height: 28px; border-radius: 8px; }
  .error { display: flex; gap: 12px; align-items: center; justify-content: space-between; color: #dc2626; font-size: 14px; }
  .skeleton div { height: 16px; border-radius: 6px; margin: 8px 0; background: linear-gradient(90deg, var(--surface-2), var(--border), var(--surface-2)); background-size: 200% 100%; animation: sh 1.1s infinite linear; }
  .skeleton div:last-child { width: 60%; }
  @keyframes sh { to { background-position: -200% 0; } }
</style>
