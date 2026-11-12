<script lang="ts">
  import { fade, fly } from 'svelte/transition';
  import { app } from './lib/state.svelte';
  import Library from './lib/components/Library.svelte';
  import Reader from './lib/components/Reader.svelte';
  import Settings from './lib/components/Settings.svelte';

  app.load();

  let systemDark = $state(window.matchMedia('(prefers-color-scheme: dark)').matches);
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', (e) => (systemDark = e.matches));

  $effect(() => {
    document.documentElement.style.setProperty(
      '--font-jp',
      app.settings.fontFamily === 'sans' ? 'var(--font-sans)' : 'var(--font-serif)',
    );
  });

  $effect(() => {
    const t = app.settings.theme;
    document.documentElement.dataset.theme = t === 'system' ? (systemDark ? 'dark' : 'light') : t;
  });
</script>

{#if app.loaded}
  {#key app.view.name === 'reader' ? `r${app.view.bookId}` : app.view.name}
    <main class="view" in:fade={{ duration: 180, delay: 60 }} out:fade={{ duration: 100 }}>
      {#if app.view.name === 'library'}
        <Library />
      {:else if app.view.name === 'reader'}
        <Reader bookId={app.view.bookId} />
      {:else}
        <Settings />
      {/if}
    </main>
  {/key}
{/if}

<div class="toasts" aria-live="polite">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}" in:fly={{ y: 16, duration: 200 }} out:fade={{ duration: 150 }}>{t.text}</div>
  {/each}
</div>

<style>
  .view { position: fixed; inset: 0; display: flex; flex-direction: column; }
  .toasts {
    position: fixed; left: 50%; bottom: calc(24px + env(safe-area-inset-bottom)); transform: translateX(-50%);
    display: flex; flex-direction: column; gap: 8px; z-index: 100; pointer-events: none; width: min(92vw, 480px);
  }
  .toast {
    background: var(--text); color: var(--bg); padding: 10px 16px; border-radius: 12px; box-shadow: var(--shadow);
    font-size: 14px; text-align: center;
  }
  .toast.error { background: #b91c1c; color: #fff; }
</style>
