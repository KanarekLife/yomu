<script lang="ts">
  import { onMount } from 'svelte';
  import { slide } from 'svelte/transition';
  import { api, errorMessage } from '../api';
  import { app, LLM_PRESETS } from '../state.svelte';
  import type { DictionaryInfo, Theme } from '../types';
  import Icon from './Icon.svelte';
  import DictionarySetup from './DictionarySetup.svelte';

  const s = app.settings;
  let moreDicts = $state(false);
  let customUrl = $state('');
  let models = $state<string[]>([]);
  let testing = $state(false);
  let testResult = $state<{ ok: boolean; text: string } | null>(null);
  let showKey = $state(false);
  let advanced = $state(false);

  const themes: { id: Theme; label: string }[] = [
    { id: 'system', label: 'System' },
    { id: 'light', label: 'Light' },
    { id: 'sepia', label: 'Sepia' },
    { id: 'dark', label: 'Dark' },
  ];

  const save = () => app.save();

  async function move(i: number, d: -1 | 1) {
    const list = [...app.dicts];
    const j = i + d;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    app.dicts = list;
    await api.reorderDictionaries(list.map((x) => x.id));
  }

  async function removeDict(d: DictionaryInfo) {
    if (!confirm(`Delete “${d.title}”?`)) return;
    await api.deleteDictionary(d.id).catch((e) => app.toast(errorMessage(e), 'error'));
    app.refreshDicts();
  }

  function preset(name: string) {
    const p = LLM_PRESETS.find((x) => x.name === name);
    if (!p) return;
    s.llm.base_url = p.base_url;
    if (p.model) s.llm.model = p.model;
    models = [];
    testResult = null;
    save();
  }

  async function test() {
    testing = true;
    testResult = null;
    try {
      const r = await api.testLlm($state.snapshot(s.llm));
      testResult = { ok: true, text: `Connected — model replied “${r.slice(0, 40)}”` };
    } catch (e) {
      testResult = { ok: false, text: errorMessage(e) };
    } finally {
      testing = false;
    }
  }

  async function fetchModels() {
    try {
      models = await api.listModels($state.snapshot(s.llm));
      if (!models.length) app.toast('The server returned no models');
    } catch (e) {
      app.toast(errorMessage(e), 'error');
    }
  }

  const extraValid = $derived.by(() => {
    if (!s.llm.extra_body.trim()) return true;
    try {
      const v = JSON.parse(s.llm.extra_body);
      return v && typeof v === 'object' && !Array.isArray(v);
    } catch {
      return false;
    }
  });

  function close() {
    if (app.view.name === 'settings') app.go(app.view.from);
  }

  onMount(() => app.refreshDicts());
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && close()} />

<header class="top">
  <button class="icon-btn" onclick={close} title="Back"><Icon name="back" /></button>
  <h1>Settings</h1>
</header>

<div class="scroll">
  <div class="col">
    <section>
      <h2>Appearance</h2>
      <div class="row">
        <span class="label">Theme</span>
        <div class="seg">
          {#each themes as t}
            <button class:on={s.theme === t.id} onclick={() => { s.theme = t.id; save(); }}>{t.label}</button>
          {/each}
        </div>
      </div>
      <div class="row">
        <span class="label">Font</span>
        <div class="seg">
          <button class="jp" class:on={s.fontFamily === 'serif'} onclick={() => { s.fontFamily = 'serif'; save(); }}>明朝 Serif</button>
          <button class:on={s.fontFamily === 'sans'} onclick={() => { s.fontFamily = 'sans'; save(); }} style="font-family:var(--font-sans)">ゴシック Sans</button>
        </div>
      </div>
      <div class="row">
        <span class="label">Text size <span class="muted">{s.fontSize}px</span></span>
        <input type="range" min="18" max="56" step="1" bind:value={s.fontSize} oninput={save} />
      </div>
      <div class="preview card" style="font-size:{s.fontSize}px">
        <ruby>吾輩<rt>わがはい</rt></ruby>は<ruby>猫<rt>ねこ</rt></ruby>である。
      </div>
    </section>

    <section>
      <h2>Translation (OpenAI-compatible API)</h2>
      <div class="presets">
        {#each LLM_PRESETS as p}
          <button class="chip" class:on={s.llm.base_url === p.base_url} onclick={() => preset(p.name)} title={p.note ?? p.base_url}>{p.name}</button>
        {/each}
      </div>
      <label class="field"><span>Base URL</span>
        <input class="input" bind:value={s.llm.base_url} oninput={save} placeholder="https://api.deepseek.com" spellcheck="false" autocapitalize="off" />
      </label>
      <label class="field"><span>API key</span>
        <div class="inline">
          <input class="input" type={showKey ? 'text' : 'password'} bind:value={s.llm.api_key} oninput={save} placeholder="sk-… (empty for local servers)" spellcheck="false" autocapitalize="off" autocomplete="off" />
          <button class="btn" onclick={() => (showKey = !showKey)}>{showKey ? 'Hide' : 'Show'}</button>
        </div>
      </label>
      <label class="field"><span>Model</span>
        <div class="inline">
          <input class="input" bind:value={s.llm.model} oninput={save} list="models" placeholder="deepseek-flash" spellcheck="false" autocapitalize="off" />
          <button class="btn" onclick={fetchModels} title="Fetch available models">List</button>
        </div>
        <datalist id="models">{#each models as m}<option value={m}></option>{/each}</datalist>
      </label>
      {#if models.length}
        <div class="models" transition:slide={{ duration: 160 }}>
          {#each models.slice(0, 60) as m}
            <button class="chip" class:on={s.llm.model === m} onclick={() => { s.llm.model = m; save(); }}>{m}</button>
          {/each}
        </div>
      {/if}
      <label class="field"><span>Translate into</span>
        <input class="input" bind:value={s.llm.target_language} oninput={save} placeholder="English" />
      </label>
      <div class="inline">
        <button class="btn" onclick={test} disabled={testing}>{testing ? 'Testing…' : 'Test connection'}</button>
        <button class="btn" onclick={() => (advanced = !advanced)}><Icon name={advanced ? 'up' : 'down'} size={16} />Advanced</button>
      </div>
      {#if testResult}
        <p class="result" class:ok={testResult.ok} transition:slide={{ duration: 150 }}>{testResult.text}</p>
      {/if}
      {#if advanced}
        <div class="adv" transition:slide={{ duration: 200 }}>
          <div class="row">
            <span class="label">Context sentences <span class="muted">{s.llm.context_sentences}</span></span>
            <input type="range" min="0" max="12" bind:value={s.llm.context_sentences} oninput={save} />
          </div>
          <div class="row">
            <span class="label">Temperature <span class="muted">{s.llm.temperature.toFixed(1)}</span></span>
            <input type="range" min="0" max="1.5" step="0.1" bind:value={s.llm.temperature} oninput={save} />
          </div>
          <div class="row">
            <span class="label">Timeout <span class="muted">{s.llm.timeout_secs}s</span></span>
            <input type="range" min="10" max="300" step="10" bind:value={s.llm.timeout_secs} oninput={save} />
          </div>
          <label class="switch"><input type="checkbox" bind:checked={s.llm.json_mode} onchange={save} /><span>Request JSON mode (<code>response_format</code>) — disable if your server rejects it</span></label>
          <label class="field"><span>Extra request JSON <span class="muted">merged into the request body</span></span>
            <textarea class="input mono" class:bad={!extraValid} rows="3" bind:value={s.llm.extra_body} oninput={save} placeholder={'{"thinking": {"type": "disabled"}}'} spellcheck="false"></textarea>
          </label>
          <label class="field"><span>Custom instructions</span>
            <textarea class="input" rows="3" bind:value={s.llm.custom_instructions} oninput={save} placeholder="e.g. Keep honorifics like -san untranslated. Explain grammar using JLPT levels."></textarea>
          </label>
        </div>
      {/if}
      <p class="hint muted">Each sentence is sent with a few preceding sentences as context. The reply contains a translation, a literal rendering, a word-by-word breakdown and grammar notes. Results are cached on this device.</p>
    </section>

    <section>
      <h2>Dictionaries</h2>
      <DictionarySetup />

      {#if app.dicts.length}
        <ul class="dicts">
          {#each app.dicts as d, i (d.id)}
            <li class="card">
              <label class="switch tight"><input type="checkbox" checked={d.enabled} onchange={(e) => api.setDictionaryEnabled(d.id, e.currentTarget.checked).then(() => app.refreshDicts())} /></label>
              <div class="dmeta">
                <div class="dtitle">{d.title}</div>
                <div class="muted small">
                  {d.term_count ? `${d.term_count.toLocaleString()} terms` : ''}{d.term_count && d.meta_count ? ' · ' : ''}{d.meta_count ? `${d.meta_count.toLocaleString()} freq/pitch` : ''}{d.revision ? ` · ${d.revision}` : ''}
                </div>
              </div>
              <button class="icon-btn sm" disabled={i === 0} onclick={() => move(i, -1)} title="Higher priority"><Icon name="up" size={16} /></button>
              <button class="icon-btn sm" disabled={i === app.dicts.length - 1} onclick={() => move(i, 1)} title="Lower priority"><Icon name="down" size={16} /></button>
              <button class="icon-btn sm danger" onclick={() => removeDict(d)} title="Delete"><Icon name="trash" size={16} /></button>
            </li>
          {/each}
        </ul>
        <p class="hint muted">Toggle to enable/disable. Higher in the list = shown first.</p>
      {/if}

      <button class="btn" onclick={() => (moreDicts = !moreDicts)}>
        <Icon name={moreDicts ? 'up' : 'down'} size={16} />More dictionaries & custom sources
      </button>
      {#if moreDicts}
        <div class="more" transition:slide={{ duration: 200 }}>
          <ul class="catalog">
            {#each app.catalog as c (c.id)}
              <li>
                <div class="dmeta">
                  <div class="dtitle">{c.name} <span class="kind">{c.kind}</span></div>
                  <div class="muted small">{c.description} · ~{c.size_mb} MB</div>
                </div>
                {#if app.isInstalled(c)}
                  <span class="installed">Installed</span>
                {:else}
                  <button class="btn" disabled={!!app.dictJob} onclick={() => app.installCatalog([c])}>Install</button>
                {/if}
              </li>
            {/each}
          </ul>
          <label class="field"><span>Install from URL <span class="muted">(direct link to a Yomitan .zip)</span></span>
            <div class="inline">
              <input class="input" bind:value={customUrl} placeholder="https://…/dictionary.zip" spellcheck="false" autocapitalize="off" />
              <button class="btn" disabled={!!app.dictJob || !customUrl.trim()} onclick={() => app.installUrl(customUrl).then(() => (customUrl = ''))}>Install</button>
            </div>
          </label>
          <div>
            <button class="btn" disabled={!!app.dictJob} onclick={() => app.importDictionaryFile()}><Icon name="plus" size={18} />Import .zip file…</button>
          </div>
          <p class="hint muted">Any Yomitan-format dictionary works: definitions (term banks), frequency and pitch accent (term meta banks).</p>
        </div>
      {/if}
    </section>

    <section class="about muted small">
      <p><b>Yomu</b> 0.1.0 · Tokenizer: lindera + IPADIC · Dictionaries: Yomitan format</p>
    </section>
  </div>
</div>

<style>
  .top { display: flex; align-items: center; gap: 6px; padding: calc(8px + env(safe-area-inset-top)) 10px 6px; }
  h1 { font-size: 22px; margin: 0; }
  .scroll { flex: 1; overflow-y: auto; padding: 0 16px calc(40px + env(safe-area-inset-bottom)); }
  .col { max-width: 680px; margin: 0 auto; display: flex; flex-direction: column; gap: 14px; }
  section { padding: 14px 0; border-bottom: 1px solid var(--border); display: flex; flex-direction: column; gap: 12px; }
  h2 { font-size: 13px; text-transform: uppercase; letter-spacing: 0.07em; color: var(--muted); margin: 0; }
  .row { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  .label { font-size: 15px; }
  .row input[type='range'] { flex: 1; max-width: 320px; accent-color: var(--accent); }
  .seg { display: inline-flex; background: var(--surface-2); padding: 3px; border-radius: 11px; gap: 2px; flex-wrap: wrap; }
  .seg button { border: none; background: transparent; padding: 7px 12px; border-radius: 8px; font-size: 14px; transition: background 150ms, box-shadow 150ms; }
  .seg button.on { background: var(--surface); box-shadow: 0 1px 3px #0002; }
  .preview { padding: 18px; text-align: center; font-family: var(--font-jp); line-height: 2; transition: font-size 120ms; }
  .preview rt { font-size: 0.45em; color: var(--muted); }
  .switch { display: flex; align-items: center; gap: 12px; font-size: 15px; cursor: pointer; }
  .switch input {
    appearance: none; width: 42px; height: 24px; border-radius: 12px; background: var(--border); position: relative;
    flex-shrink: 0; cursor: pointer; transition: background 180ms var(--ease); margin: 0;
  }
  .switch input::after {
    content: ''; position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: #fff;
    box-shadow: 0 1px 2px #0003; transition: transform 180ms var(--ease);
  }
  .switch input:checked { background: var(--accent); }
  .switch input:checked::after { transform: translateX(18px); }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 14px; }
  .inline { display: flex; gap: 8px; flex-wrap: wrap; }
  .inline .input { flex: 1; min-width: 0; }
  .presets, .models { display: flex; flex-wrap: wrap; gap: 6px; }
  .models { max-height: 160px; overflow-y: auto; }
  .chip { border: 1px solid var(--border); background: var(--surface); border-radius: 999px; padding: 6px 12px; font-size: 13px; transition: all 150ms; }
  .chip.on { border-color: var(--accent); background: var(--accent-soft); color: var(--accent); }
  .mono { font-family: ui-monospace, monospace; font-size: 13px; }
  .bad { border-color: #dc2626; }
  textarea.input { resize: vertical; }
  .adv { display: flex; flex-direction: column; gap: 12px; }
  .result { margin: 0; font-size: 14px; color: #dc2626; word-break: break-word; }
  .result.ok { color: #16a34a; }
  .hint { font-size: 13px; margin: 0; line-height: 1.5; }
  code { font-size: 12px; background: var(--surface-2); padding: 1px 5px; border-radius: 4px; }
  .dicts { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
  .dicts li { display: flex; align-items: center; gap: 6px; padding: 8px 8px 8px 12px; }
  .dmeta { flex: 1; min-width: 0; }
  .dtitle { font-weight: 600; font-size: 15px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .small { font-size: 12px; }
  .icon-btn.sm { width: 34px; height: 34px; border-radius: 9px; }
  .danger { color: #dc2626; }
  .more { display: flex; flex-direction: column; gap: 14px; }
  .catalog { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; }
  .catalog li { display: flex; align-items: center; gap: 12px; padding: 10px 0; border-bottom: 1px solid var(--border); }
  .catalog .dtitle { white-space: normal; }
  .kind { font-size: 11px; font-weight: 500; color: var(--muted); background: var(--surface-2); padding: 2px 7px; border-radius: 999px; margin-left: 4px; }
  .installed { font-size: 13px; color: #16a34a; white-space: nowrap; }
  .about { border: none; }
</style>
