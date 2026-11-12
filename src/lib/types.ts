// Mirrors the serde types of `reader-core`. Offsets are char (code point)
// indices — see ADR-0006.

export interface Seg {
  text: string;
  ruby?: string;
}

export interface Token {
  surface: string;
  start: number;
  end: number;
  segs: Seg[];
  base?: string;
  reading?: string;
  pos: string;
  punct: boolean;
}

export type SyncMode = 'off' | 'server' | 'folder';

export interface SyncStatus {
  mode: SyncMode;
  server_url: string;
  username: string;
  signed_in: boolean;
  folder: string;
  device_name: string;
  device_id: string;
  last_sync_at: number | null;
  last_error: string | null;
}

export interface SyncReport {
  books_added: number;
  books_updated: number;
  books_deleted: number;
  settings_changed: boolean;
  translations: number;
  files_uploaded: number;
  files_downloaded: number;
  waiting_for_files: number;
}

export interface SentenceView {
  idx: number;
  chapter: number;
  paragraph_start: boolean;
  text: string;
  tokens: Token[];
  /** Illustration step: path of the image inside the EPUB (`text` is empty). */
  image: string | null;
}

export interface BookSummary {
  id: number;
  title: string;
  author: string;
  sentence_count: number;
  position: number;
  added_at: number;
  last_read_at: number | null;
  has_cover: boolean;
  outdated: boolean;
}

export interface Chapter {
  idx: number;
  title: string;
  start: number;
  /** 0 = table-of-contents entry, 1 = heading found in the text */
  level: number;
}

export interface BookDetail extends BookSummary {
  chapters: Chapter[];
}

export interface Tag {
  name: string;
  category: string;
  notes: string;
}

export interface Definition {
  dictionary: string;
  tags: Tag[];
  glossary: unknown;
}

export interface TermEntry {
  expression: string;
  reading: string;
  source: string;
  length: number;
  reasons: string[];
  term_tags: Tag[];
  definitions: Definition[];
  frequencies: { dictionary: string; value: string }[];
  pitches: { dictionary: string; reading: string; positions: number[] }[];
}

export interface LookupResult {
  entries: TermEntry[];
  length: number;
  has_dictionaries: boolean;
}

export interface DictionaryInfo {
  id: number;
  title: string;
  revision: string;
  enabled: boolean;
  priority: number;
  term_count: number;
  meta_count: number;
  source: string;
}

export interface CatalogEntry {
  id: string;
  name: string;
  kind: string;
  description: string;
  url: string;
  size_mb: number;
  recommended: boolean;
}

export interface ImportProgress {
  name: string;
  stage: string;
  done: number;
  total: number;
}

export interface LlmConfig {
  base_url: string;
  api_key: string;
  model: string;
  target_language: string;
  temperature: number;
  context_sentences: number;
  json_mode: boolean;
  extra_body: string;
  custom_instructions: string;
  timeout_secs: number;
}

export interface Translation {
  translation: string;
  literal: string;
  breakdown: { text: string; reading: string; meaning: string; role: string }[];
  grammar: { pattern: string; explanation: string }[];
  notes: string;
  model: string;
  cached: boolean;
}

export type Theme = 'system' | 'light' | 'dark' | 'sepia';

export interface Settings {
  theme: Theme;
  fontFamily: 'serif' | 'sans';
  fontSize: number;
  furigana: boolean;
  translation: boolean;
  prefetchTranslation: boolean;
  showBreakdown: boolean;
  showContext: boolean;
  showImages: boolean;
  llm: LlmConfig;
}
