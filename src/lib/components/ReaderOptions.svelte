<script lang="ts">
  // Quick reading options, reachable without leaving the book (ADR-0009).
  import { fade, fly } from 'svelte/transition';
  import { cubicOut } from 'svelte/easing';
  import { app } from '../state.svelte';
  import type { Theme } from '../types';

  let { onclose }: { onclose: () => void } = $props();
  const s = app.settings;
  const save = () => app.save();
  const themes: { id: Theme; label: string; swatch: string }[] = [
    { id: 'system', label: 'Auto', swatch: 'linear-gradient(135deg,#f7f6f3 50%,#141413 50%)' },
    { id: 'light', label: 'Light', swatch: '#f7f6f3' },
    { id: 'sepia', label: 'Sepia', swatch: '#f4ecd8' },
    { id: 'dark', label: 'Dark', swatch: '#141413' },
  ];
  const size = (d: number) => {
    s.fontSize = Math.max(16, Math.min(60, s.fontSize + d));
    save();
  };
  const toggles: { key: 'furigana' | 'translation' | 'showBreakdown' | 'prefetchTranslation' | 'showContext'; label: string; hint?: string }[] = [
    { key: 'furigana', label: 'Furigana', hint: 'F' },
    { key: 'translation', label: 'Translation', hint: 'T' },
    { key: 'showBreakdown', label: 'Expand sentence breakdown' },
    { key: 'prefetchTranslation', label: 'Pre-translate next sentence' },
    { key: 'showContext', label: 'Show previous sentence' },
  ];
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="scrim" transition:fade={{ duration: 150 }} onclick={onclose}></div>
<div class="sheet card" transition:fly={{ y: 40, duration: 220, easing: cubicOut }} role="dialog" aria-label="Reading options">
  <div class="sizes">
    <button class="btn size" onclick={() => size(-2)} aria-label="Smaller text"><span class="jp" style="font-size:14px">あ</span></button>
    <input type="range" min="16" max="60" step="1" bind:value={s.fontSize} oninput={save} aria-label="Text size" />
    <button class="btn size" onclick={() => size(2)} aria-label="Larger text"><span class="jp" style="font-size:22px">あ</span></button>
  </div>
  <div class="seg">
    <button class:on={s.fontFamily === 'serif'} onclick={() => { s.fontFamily = 'serif'; save(); }} style="font-family:var(--font-serif)">明朝</button>
    <button class:on={s.fontFamily === 'sans'} onclick={() => { s.fontFamily = 'sans'; save(); }} style="font-family:var(--font-sans)">ゴシック</button>
  </div>
  <div class="themes">
    {#each themes as t}
      <button class="theme" class:on={s.theme === t.id} onclick={() => { s.theme = t.id; save(); }} title={t.label}>
        <span class="swatch" style="background:{t.swatch}"></span><span>{t.label}</span>
      </button>
    {/each}
  </div>
  <div class="toggles">
    {#each toggles as t}
      <label class="switch">
        <span>{t.label}{#if t.hint}<kbd>{t.hint}</kbd>{/if}</span>
        <input type="checkbox" bind:checked={s[t.key]} onchange={save} />
      </label>
    {/each}
  </div>
</div>

<style>
  .scrim { position: fixed; inset: 0; z-index: 30; }
  .sheet {
    position: fixed; z-index: 31; right: max(12px, calc(50% - 430px)); bottom: calc(96px + env(safe-area-inset-bottom));
    width: min(360px, calc(100vw - 24px)); padding: 14px; box-shadow: var(--shadow);
    display: flex; flex-direction: column; gap: 12px;
  }
  .sizes { display: flex; align-items: center; gap: 10px; }
  .sizes input { flex: 1; accent-color: var(--accent); }
  .size { width: 44px; padding: 0; }
  .seg { display: flex; background: var(--surface-2); padding: 3px; border-radius: 11px; gap: 2px; }
  .seg button { flex: 1; border: none; background: transparent; padding: 8px; border-radius: 8px; font-size: 15px; transition: background 150ms, box-shadow 150ms; }
  .seg button.on { background: var(--surface); box-shadow: 0 1px 3px #0002; }
  .themes { display: grid; grid-template-columns: repeat(4, 1fr); gap: 6px; }
  .theme { border: 1px solid var(--border); background: transparent; border-radius: 10px; padding: 8px 4px; display: flex; flex-direction: column; align-items: center; gap: 5px; font-size: 12px; transition: border-color 150ms; }
  .theme.on { border-color: var(--accent); color: var(--accent); }
  .swatch { width: 22px; height: 22px; border-radius: 50%; border: 1px solid #8884; }
  .toggles { display: flex; flex-direction: column; border-top: 1px solid var(--border); padding-top: 4px; }
  .switch { display: flex; align-items: center; justify-content: space-between; gap: 12px; font-size: 14px; cursor: pointer; padding: 7px 0; }
  kbd { font-family: var(--font-ui); font-size: 10px; border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; margin-left: 6px; color: var(--muted); }
  .switch input {
    appearance: none; width: 40px; height: 23px; border-radius: 12px; background: var(--border); position: relative;
    flex-shrink: 0; cursor: pointer; transition: background 180ms var(--ease); margin: 0;
  }
  .switch input::after {
    content: ''; position: absolute; top: 3px; left: 3px; width: 17px; height: 17px; border-radius: 50%; background: #fff;
    box-shadow: 0 1px 2px #0003; transition: transform 180ms var(--ease);
  }
  .switch input:checked { background: var(--accent); }
  .switch input:checked::after { transform: translateX(17px); }
</style>
