import { listen } from '@tauri-apps/api/event';
import { api, errorMessage } from './api';
import type { CatalogEntry, DictionaryInfo, ImportProgress, LlmConfig, Settings, SyncReport, SyncStatus } from './types';

export const LLM_PRESETS: { name: string; base_url: string; model: string; note?: string }[] = [
  { name: 'DeepSeek', base_url: 'https://api.deepseek.com', model: 'deepseek-flash' },
  { name: 'OpenAI', base_url: 'https://api.openai.com/v1', model: 'gpt-5-mini' },
  { name: 'OpenRouter', base_url: 'https://openrouter.ai/api/v1', model: 'deepseek/deepseek-v4-flash' },
  { name: 'Ollama (local)', base_url: 'http://localhost:11434/v1', model: 'qwen3:8b', note: 'No API key needed' },
  { name: 'LM Studio (local)', base_url: 'http://localhost:1234/v1', model: '', note: 'No API key needed' },
];

export const DEFAULT_LLM: LlmConfig = {
  base_url: 'https://api.deepseek.com',
  api_key: '',
  model: 'deepseek-flash',
  target_language: 'English',
  temperature: 0.3,
  context_sentences: 4,
  json_mode: true,
  extra_body: '',
  custom_instructions: '',
  timeout_secs: 60,
};

export const DEFAULT_SETTINGS: Settings = {
  theme: 'system',
  fontFamily: 'serif',
  fontSize: 30,
  furigana: true,
  translation: false,
  prefetchTranslation: true,
  showBreakdown: true,
  showContext: false,
  showImages: true,
  llm: DEFAULT_LLM,
};

type View = { name: 'library' } | { name: 'reader'; bookId: number } | { name: 'settings'; from: View };

class AppState {
  view = $state<View>({ name: 'library' });
  settings = $state<Settings>(structuredClone(DEFAULT_SETTINGS));
  loaded = $state(false);
  toasts = $state<{ id: number; text: string; kind: 'info' | 'error' }[]>([]);
  /** Installed dictionaries + catalog, shared by every view. */
  dicts = $state<DictionaryInfo[]>([]);
  catalog = $state<CatalogEntry[]>([]);
  /** Running dictionary install job (download/import), if any. */
  dictJob = $state<{ queue: string[]; current: string; progress: ImportProgress | null } | null>(null);
  /** Sync (ADR-0016). */
  sync = $state<SyncStatus | null>(null);
  syncing = $state(false);
  #saveTimer: ReturnType<typeof setTimeout> | undefined;
  #syncTimer: ReturnType<typeof setTimeout> | undefined;
  #toastId = 0;

  async #readSettings() {
    const stored = (await api.getSettings()) as Partial<Settings> | null;
    if (stored && typeof stored === 'object') {
      // Mutate in place: components hold references to `settings` / `settings.llm`.
      Object.assign(this.settings, { ...structuredClone(DEFAULT_SETTINGS), ...stored, llm: this.settings.llm });
      Object.assign(this.settings.llm, { ...DEFAULT_LLM, ...(stored.llm ?? {}) });
    }
  }

  async load() {
    try {
      await this.#readSettings();
    } catch (e) {
      this.toast(errorMessage(e), 'error');
    }
    this.loaded = true;
    // Another device's settings arrived through sync.
    listen('settings-changed', () => this.#readSettings().catch(() => {}));
    api.syncStatus().then((s) => {
      this.sync = s;
      this.runSync();
    }).catch(() => {});
    setInterval(() => this.runSync(), 5 * 60_000);
    document.addEventListener('visibilitychange', () => {
      if (document.visibilityState === 'hidden') this.runSync();
      else this.requestSync(1000);
    });
    listen<ImportProgress>('dict-import-progress', (e) => {
      if (this.dictJob) this.dictJob.progress = e.payload;
    });
    this.refreshDicts();
    api.dictionaryCatalog().then((c) => (this.catalog = c)).catch(() => {});
  }

  async refreshDicts() {
    try {
      this.dicts = await api.listDictionaries();
    } catch (e) {
      this.toast(errorMessage(e), 'error');
    }
  }

  isInstalled(entry: CatalogEntry) {
    return this.dicts.some((d) => d.source === entry.id);
  }

  get recommendedMissing() {
    return this.catalog.filter((c) => c.recommended && !this.isInstalled(c));
  }

  async #run(names: string[], steps: (() => Promise<string | null>)[]) {
    if (this.dictJob) return;
    this.dictJob = { queue: names, current: names[0] ?? '', progress: null };
    const done: string[] = [];
    try {
      for (let i = 0; i < steps.length; i++) {
        this.dictJob.current = names[i];
        this.dictJob.progress = null;
        try {
          const r = await steps[i]();
          if (r) done.push(r);
        } catch (e) {
          this.toast(`${names[i]}: ${errorMessage(e)}`, 'error');
        }
        await this.refreshDicts();
      }
    } finally {
      this.dictJob = null;
    }
    if (done.length) this.toast(`Installed ${done.join(', ')}`);
  }

  /** One-click install of catalog entries (skips ones already installed). */
  installCatalog(entries: CatalogEntry[]) {
    const todo = entries.filter((e) => !this.isInstalled(e));
    return this.#run(
      todo.map((e) => e.name),
      todo.map((e) => async () => (await api.installDictionaryUrl(e.url, e.id, e.name)).title),
    );
  }

  installUrl(url: string) {
    return this.#run(['Custom dictionary'], [async () => (await api.installDictionaryUrl(url, '', 'Custom dictionary')).title]);
  }

  importDictionaryFile() {
    return this.#run(['Dictionary file'], [async () => (await api.importDictionary())?.title ?? null]);
  }

  /** Debounced persistence; call after mutating `settings`. */
  save() {
    clearTimeout(this.#saveTimer);
    this.#saveTimer = setTimeout(() => {
      api.setSettings($state.snapshot(this.settings)).catch((e) => this.toast(errorMessage(e), 'error'));
      this.requestSync(15_000);
    }, 300);
  }

  /** Sync soon (debounced), e.g. after importing a book or leaving the reader. */
  requestSync(delay = 2000) {
    if (!this.sync || this.sync.mode === 'off') return;
    clearTimeout(this.#syncTimer);
    this.#syncTimer = setTimeout(() => this.runSync(), delay);
  }

  /** One sync now. `manual` shows the outcome; background syncs stay quiet. */
  async runSync(manual = false): Promise<SyncReport | null> {
    if (!this.sync || this.sync.mode === 'off' || this.syncing) return null;
    if (this.sync.mode === 'server' && !this.sync.signed_in) return null;
    clearTimeout(this.#syncTimer);
    this.syncing = true;
    let rep: SyncReport | null = null;
    try {
      rep = await api.syncNow();
      if (manual && rep) this.toast(describeSync(rep));
    } catch (e) {
      if (manual) this.toast(errorMessage(e), 'error');
    } finally {
      this.syncing = false;
      this.sync = await api.syncStatus().catch(() => this.sync);
    }
    return rep;
  }

  toast(text: string, kind: 'info' | 'error' = 'info') {
    const id = ++this.#toastId;
    this.toasts.push({ id, text, kind });
    setTimeout(() => (this.toasts = this.toasts.filter((t) => t.id !== id)), kind === 'error' ? 6000 : 3000);
  }

  go(view: View) {
    this.view = view;
  }
}

export const app = new AppState();

function describeSync(r: SyncReport): string {
  const parts: string[] = [];
  if (r.books_added) parts.push(`${r.books_added} book${r.books_added > 1 ? 's' : ''} added`);
  if (r.books_updated) parts.push(`${r.books_updated} updated`);
  if (r.books_deleted) parts.push(`${r.books_deleted} removed`);
  if (r.settings_changed) parts.push('settings updated');
  if (r.translations) parts.push(`${r.translations} translations`);
  if (r.files_uploaded) parts.push(`${r.files_uploaded} uploaded`);
  if (r.waiting_for_files) parts.push(`${r.waiting_for_files} waiting for their EPUB`);
  return parts.length ? `Synced: ${parts.join(', ')}` : 'Synced — everything up to date';
}
