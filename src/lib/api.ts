import { invoke } from '@tauri-apps/api/core';
import type {
  BookDetail,
  BookSummary,
  CatalogEntry,
  DictionaryInfo,
  LlmConfig,
  LookupResult,
  SentenceView,
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
};

export function errorMessage(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
