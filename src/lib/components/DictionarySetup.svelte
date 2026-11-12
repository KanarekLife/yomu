<script lang="ts">
  // One-click "recommended dictionaries" card + live install progress (ADR-0011).
  import { slide } from 'svelte/transition';
  import { app } from '../state.svelte';
  import Icon from './Icon.svelte';

  let { compact = false }: { compact?: boolean } = $props();

  const recommended = $derived(app.catalog.filter((c) => c.recommended));
  const missing = $derived(app.recommendedMissing);
  const size = $derived(missing.reduce((a, c) => a + c.size_mb, 0));
  const job = $derived(app.dictJob);
  const p = $derived(job?.progress);
  const frac = $derived(p && p.total ? Math.min(1, p.done / p.total) : 0);
  const mb = (b: number) => (b / 1048576).toFixed(1);
  const stageText = $derived(
    !p
      ? 'Starting…'
      : p.stage === 'downloading'
        ? `Downloading ${mb(p.done)}${p.total ? ` / ${mb(p.total)}` : ''} MB`
        : `Installing ${p.stage} (${p.done}/${p.total})`,
  );
</script>

{#if job}
  <div class="setup card" class:compact transition:slide={{ duration: 180 }}>
    <div class="row">
      <div class="spinner"></div>
      <div class="grow">
        <div class="title">{job.current}{job.queue.length > 1 ? ` · ${job.queue.indexOf(job.current) + 1}/${job.queue.length}` : ''}</div>
        <div class="muted small">{stageText}</div>
      </div>
    </div>
    <div class="bar"><div style="transform:scaleX({frac})"></div></div>
  </div>
{:else if missing.length}
  <div class="setup card" class:compact transition:slide={{ duration: 180 }}>
    <div class="title">{app.dicts.length ? 'Complete the recommended setup' : 'Set up word lookup'}</div>
    {#if !compact}
      <ul class="list">
        {#each recommended as r}
          <li class:done={app.isInstalled(r)}>
            <span class="tick">{app.isInstalled(r) ? '✓' : '•'}</span>
            <b>{r.name}</b> <span class="muted">— {r.kind}</span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted small">Jitendex definitions, JPDB frequency and pitch accents.</p>
    {/if}
    <button class="btn primary" onclick={() => app.installCatalog(missing)}>
      <Icon name="plus" size={18} />Install recommended (~{size} MB)
    </button>
  </div>
{/if}

<style>
  .setup { padding: 14px 16px; display: flex; flex-direction: column; gap: 10px; text-align: left; }
  .setup.compact { padding: 12px; }
  .title { font-weight: 600; font-size: 15px; }
  .small { font-size: 12px; margin: 0; }
  .row { display: flex; gap: 12px; align-items: center; }
  .grow { flex: 1; min-width: 0; }
  .list { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 4px; font-size: 14px; }
  .list li.done { opacity: 0.6; }
  .tick { display: inline-block; width: 16px; color: var(--accent); }
  .bar { height: 4px; background: var(--surface-2); border-radius: 2px; overflow: hidden; }
  .bar div { height: 100%; background: var(--accent); transform-origin: left; transition: transform 200ms var(--ease); }
  .spinner { width: 18px; height: 18px; border-radius: 50%; border: 2px solid var(--border); border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex-shrink: 0; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
