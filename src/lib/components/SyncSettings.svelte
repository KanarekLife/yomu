<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { android, api, errorMessage } from '../api';
  import { app } from '../state.svelte';
  import type { SyncMode } from '../types';

  // Sync settings (ADR-0016): a Yomu server, or a folder synced by Syncthing & co.
  const isAndroid = !!android;
  let mode = $state<SyncMode>('off');
  let serverUrl = $state('');
  let username = $state('');
  let password = $state('');
  let folder = $state('');
  let deviceName = $state('');
  let busy = $state(false);
  let error = $state<string | null>(null);
  let fileAccess = $state(android?.hasAllFilesAccess() ?? true);

  const st = $derived(app.sync);
  const defaultName = isAndroid ? 'Android' : navigator.platform.toLowerCase().includes('win') ? 'Windows PC' : 'Linux PC';

  onMount(() => {
    (async () => {
      const s = app.sync ?? (app.sync = await api.syncStatus());
      mode = s.mode;
      serverUrl = s.server_url;
      username = s.username;
      folder = s.folder || (isAndroid ? `${android!.storageRoot()}/Sync/Yomu` : '');
      deviceName = s.device_name || defaultName;
    })().catch((e) => (error = errorMessage(e)));
    // Returning from Android's permission screen.
    const vis = () => (fileAccess = android?.hasAllFilesAccess() ?? true);
    document.addEventListener('visibilitychange', vis);
    return () => document.removeEventListener('visibilitychange', vis);
  });

  async function act(f: () => Promise<unknown>) {
    busy = true;
    error = null;
    try {
      await f();
    } catch (e) {
      error = errorMessage(e);
    } finally {
      busy = false;
    }
  }

  const setMode = (m: SyncMode) =>
    act(async () => {
      mode = m;
      // Server mode is switched on by signing in.
      if (m === 'off' || (m === 'server' && st?.signed_in)) app.sync = await api.syncConfigure(m, st?.folder ?? '', deviceName);
      // Folder mode starts with "Use this folder".
    });

  const signIn = () =>
    act(async () => {
      app.sync = await api.syncLogin(serverUrl, username, password, deviceName);
      password = '';
      await app.runSync(true);
    });

  const signOut = () =>
    act(async () => {
      app.sync = await api.syncLogout();
    });

  const useFolder = () =>
    act(async () => {
      if (isAndroid && !fileAccess) throw new Error('Allow “All files access” first.');
      app.sync = await api.syncConfigure('folder', folder, deviceName);
      await app.runSync(true);
    });

  async function browse() {
    const p = await api.pickFolder().catch(() => null);
    if (p) folder = p;
  }

  const when = (ms: number | null | undefined) => (ms ? new Date(ms).toLocaleString() : 'never');
</script>

<div class="seg" role="tablist">
  <button class:on={mode === 'off'} onclick={() => setMode('off')}>Off</button>
  <button class:on={mode === 'server'} onclick={() => setMode('server')}>Yomu server</button>
  <button class:on={mode === 'folder'} onclick={() => setMode('folder')}>Folder (Syncthing)</button>
</div>

{#if mode !== 'off'}
  <div class="body" transition:slide={{ duration: 180 }}>
    <p class="hint muted">
      Syncs your library (including the EPUB files), reading positions, settings — <b>including the API key</b> — and cached translations.
    </p>

    {#if mode === 'server'}
      {#if st?.mode === 'server' && st.signed_in}
        <p class="status">Signed in as <b>{st.username}</b> on <span class="mono">{st.server_url}</span></p>
        <div class="inline">
          <button class="btn primary" disabled={app.syncing} onclick={() => app.runSync(true)}>{app.syncing ? 'Syncing…' : 'Sync now'}</button>
          <button class="btn" disabled={busy} onclick={signOut}>Sign out</button>
        </div>
      {:else}
        <label class="field"><span>Server</span>
          <input class="input" bind:value={serverUrl} placeholder="https://yomu.example.com" spellcheck="false" autocapitalize="off" inputmode="url" />
        </label>
        <div class="two">
          <label class="field"><span>Username</span>
            <input class="input" bind:value={username} autocapitalize="off" spellcheck="false" autocomplete="username" />
          </label>
          <label class="field"><span>Password</span>
            <input class="input" type="password" bind:value={password} autocomplete="current-password" onkeydown={(e) => e.key === 'Enter' && signIn()} />
          </label>
        </div>
        <label class="field"><span>This device</span>
          <input class="input" bind:value={deviceName} placeholder={defaultName} />
        </label>
        <div class="inline">
          <button class="btn primary" disabled={busy || !serverUrl || !username || !password} onclick={signIn}>{busy ? 'Signing in…' : 'Sign in'}</button>
        </div>
        <p class="hint muted">Accounts are created by the server's admin (<span class="mono">/admin</span>).</p>
      {/if}
    {:else}
      <label class="field"><span>Folder</span>
        <div class="inline">
          <input class="input mono" bind:value={folder} placeholder="/home/me/Sync/Yomu" spellcheck="false" autocapitalize="off" />
          {#if !isAndroid}<button class="btn" onclick={browse}>Browse…</button>{/if}
        </div>
      </label>
      {#if isAndroid && !fileAccess}
        <div class="perm">
          <p>Syncthing's folders are in shared storage, which Android only lets Yomu read with <b>All files access</b>.</p>
          <button class="btn" onclick={() => android?.requestAllFilesAccess()}>Allow file access</button>
        </div>
      {/if}
      <label class="field"><span>This device</span>
        <input class="input" bind:value={deviceName} placeholder={defaultName} />
      </label>
      <div class="inline">
        {#if st?.mode === 'folder' && st.folder === folder}
          <button class="btn primary" disabled={app.syncing} onclick={() => app.runSync(true)}>{app.syncing ? 'Syncing…' : 'Sync now'}</button>
        {:else}
          <button class="btn primary" disabled={busy || !folder} onclick={useFolder}>Use this folder</button>
        {/if}
      </div>
      <p class="hint muted">
        Share this folder between your devices with Syncthing (or any file-sync tool). Each device writes only its own
        <span class="mono">devices/&lt;id&gt;.json</span> plus the EPUBs in <span class="mono">books/</span>, so there are no conflicts.
      </p>
    {/if}

    {#if st && st.mode === mode && (mode !== 'server' || st.signed_in)}
      <p class="small muted">Last sync: {when(st.last_sync_at)}{#if st.last_error} · <span class="err">{st.last_error}</span>{/if}</p>
    {/if}
    {#if error}<p class="err" transition:slide={{ duration: 150 }}>{error}</p>{/if}
  </div>
{/if}

<style>
  .seg { display: inline-flex; background: var(--surface-2); padding: 3px; border-radius: 11px; gap: 2px; flex-wrap: wrap; align-self: flex-start; }
  .seg button { border: none; background: transparent; padding: 7px 12px; border-radius: 8px; font-size: 14px; transition: background 150ms, box-shadow 150ms; }
  .seg button.on { background: var(--surface); box-shadow: 0 1px 3px #0002; }
  .body { display: flex; flex-direction: column; gap: 12px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 14px; flex: 1; min-width: 0; }
  .two { display: flex; gap: 10px; flex-wrap: wrap; }
  .two .field { min-width: 180px; }
  .inline { display: flex; gap: 8px; flex-wrap: wrap; }
  .inline .input { flex: 1; min-width: 0; }
  .hint { font-size: 13px; margin: 0; line-height: 1.5; }
  .status { margin: 0; font-size: 14px; word-break: break-word; }
  .mono { font-family: ui-monospace, monospace; font-size: 0.92em; }
  .small { font-size: 12px; margin: 0; }
  .err { color: #dc2626; margin: 0; font-size: 13px; word-break: break-word; }
  .perm { padding: 12px; border-radius: 12px; background: var(--accent-soft); font-size: 13px; display: flex; flex-direction: column; gap: 8px; align-items: flex-start; }
  .perm p { margin: 0; }
</style>
