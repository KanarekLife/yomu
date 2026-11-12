import { invoke } from '@tauri-apps/api/core';
import type {
  BookDetail,
  BookSummary,
  CatalogEntry,
  DictionaryInfo,
  LlmConfig,
  LookupResult,
  SentenceView,
  SyncMode,
  SyncReport,
  SyncStatus,
  Translation,
} from './types';

/** Typed wrappers around the Tauri commands in `src-tauri/src/lib.rs`. */
export const api = {
  listBooks: () => invoke<BookSummary[]>('list_books'),
  bookDetail: (id: number) => invoke<BookDetail>('book_detail', { id }),
  bookCover: async (id: number): Promise<string | null> => {
    const buf = await invoke<ArrayBuffer>('book_cover', { id });
    if (!buf || buf.byteLength === 0) return null;
    return URL.createObjectURL(new Blob([buf]));
  },
  /** Object URL for an illustration inside the book's EPUB. */
  bookImage: async (id: number, path: string): Promise<string> => {
    const buf = new Uint8Array(await invoke<ArrayBuffer>('book_image', { id, path }));
    const nl = buf.indexOf(10);
    const type = new TextDecoder().decode(buf.subarray(0, nl));
    return URL.createObjectURL(new Blob([buf.subarray(nl + 1)], { type }));
  },
  deleteBook: (id: number) => invoke<void>('delete_book', { id }),
  importBook: () => invoke<BookSummary | null>('import_book'),
  importBookPath: (path: string) => invoke<BookSummary>('import_book_path', { path }),
  sentences: (bookId: number, from: number, count: number) =>
    invoke<SentenceView[]>('sentences', { bookId, from, count }),
  setPosition: (bookId: number, position: number) => invoke<void>('set_position', { bookId, position }),
  lookup: (text: string, offset: number) => invoke<LookupResult>('lookup', { text, offset }),
  getSettings: () => invoke<unknown>('get_settings'),
  setSettings: (value: unknown) => invoke<void>('set_settings', { value }),
  translate: (bookId: number, idx: number, force = false) =>
    invoke<Translation>('translate', { bookId, idx, force }),
  cachedTranslation: (bookId: number, idx: number) =>
    invoke<Translation | null>('cached_translation', { bookId, idx }),
  testLlm: (config: LlmConfig) => invoke<string>('test_llm', { config }),
  listModels: (config: LlmConfig) => invoke<string[]>('list_models', { config }),
  listDictionaries: () => invoke<DictionaryInfo[]>('list_dictionaries'),
  importDictionary: () => invoke<DictionaryInfo | null>('import_dictionary'),
  dictionaryCatalog: () => invoke<CatalogEntry[]>('dictionary_catalog'),
  installDictionaryUrl: (url: string, source: string, name: string) =>
    invoke<DictionaryInfo>('install_dictionary_url', { url, source, name }),
  setDictionaryEnabled: (id: number, enabled: boolean) =>
    invoke<void>('set_dictionary_enabled', { id, enabled }),
  reorderDictionaries: (ids: number[]) => invoke<void>('reorder_dictionaries', { ids }),
  deleteDictionary: (id: number) => invoke<void>('delete_dictionary', { id }),
  syncStatus: () => invoke<SyncStatus>('sync_status'),
  syncConfigure: (mode: SyncMode, folder: string, deviceName: string) =>
    invoke<SyncStatus>('sync_configure', { mode, folder, deviceName }),
  syncLogin: (serverUrl: string, username: string, password: string, deviceName: string) =>
    invoke<SyncStatus>('sync_login', { serverUrl, username, password, deviceName }),
  pickFolder: () => invoke<string | null>('pick_folder'),
  syncLogout: () => invoke<SyncStatus>('sync_logout'),
  /** `null` when a sync was already running. */
  syncNow: () => invoke<SyncReport | null>('sync_now'),
};

/** Android-only bridge (MainActivity.kt) for folder sync in shared storage. */
type AndroidBridge = { hasAllFilesAccess(): boolean; requestAllFilesAccess(): void; storageRoot(): string };
export const android: AndroidBridge | undefined = (globalThis as { YomuAndroid?: AndroidBridge }).YomuAndroid;

export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
