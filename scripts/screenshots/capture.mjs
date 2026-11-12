// Renders README screenshots of the real UI (production build) in headless
// Chromium, with the Tauri bridge stubbed by fixtures that the real Rust core
// generated from public-domain text.
//
//   cargo run -p reader-core --release --example screenshot_fixtures -- scripts/screenshots/fixtures.json
//   npm run build && npm i --no-save playwright && npx playwright install chromium
//   node scripts/screenshots/capture.mjs
import { readFileSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { preview } from 'vite';
import { chromium } from 'playwright';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..', '..');
const outDir = join(root, 'docs', 'screenshots');
mkdirSync(outDir, { recursive: true });
const fx = JSON.parse(readFileSync(join(here, 'fixtures.json'), 'utf8'));

const server = await preview({ root, preview: { port: 4173, strictPort: true }, logLevel: 'warn' });
const url = 'http://localhost:4173/';
const browser = await chromium.launch();

// ---- covers: rendered from HTML so no copyrighted artwork is involved -------
const COVERS = [
  { title: '吾輩は猫である', author: '夏目漱石', bg: 'linear-gradient(160deg,#f6d365,#fda085)', fg: '#3b1d0b' },
  { title: '坊っちゃん', author: '夏目漱石', bg: 'linear-gradient(160deg,#a1c4fd,#5b7fd6)', fg: '#0c1b3a' },
  { title: '銀河鉄道の夜', author: '宮沢賢治', bg: 'linear-gradient(170deg,#0f2027,#2c5364)', fg: '#f5e9b8' },
  { title: '羅生門', author: '芥川龍之介', bg: 'linear-gradient(160deg,#5a1f1f,#b33939)', fg: '#fbe9e7' },
];
async function renderCover(c) {
  const p = await browser.newPage({ viewport: { width: 400, height: 600 } });
  await p.setContent(`<body style="margin:0"><div id="c" style="width:400px;height:600px;background:${c.bg};color:${c.fg};
    display:flex;flex-direction:row-reverse;justify-content:space-between;padding:56px 48px;box-sizing:border-box;
    font-family:'Noto Serif CJK JP',serif">
    <div style="writing-mode:vertical-rl;font-size:56px;font-weight:700;letter-spacing:.12em">${c.title}</div>
    <div style="writing-mode:vertical-rl;font-size:26px;align-self:flex-end;opacity:.85">${c.author}</div></div></body>`);
  const png = await p.locator('#c').screenshot();
  await p.close();
  return png.toString('base64');
}
const covers = await Promise.all(COVERS.map(renderCover));

const now = Math.floor(Date.now() / 1000);
const real = { ...fx.book, has_cover: true, position: 2, last_read_at: now };
const books = [
  real,
  { id: 101, title: '坊っちゃん', author: '夏目漱石', sentence_count: 3150, position: 2290, added_at: now - 9e5, last_read_at: now - 3e5, has_cover: true, outdated: false },
  { id: 102, title: '銀河鉄道の夜', author: '宮沢賢治', sentence_count: 2410, position: 410, added_at: now - 8e5, last_read_at: now - 6e5, has_cover: true, outdated: false },
  { id: 103, title: '羅生門', author: '芥川龍之介', sentence_count: 260, position: 259, added_at: now - 7e5, last_read_at: now - 7e5, has_cover: true, outdated: false },
];
const coverById = { [real.id]: covers[0], 101: covers[1], 102: covers[2], 103: covers[3] };

/** Installs a fake `window.__TAURI_INTERNALS__` answering every command. */
function bridge({ data, settings, position }) {
  const b64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0)).buffer;
  let cb = 1;
  window.__TAURI_INTERNALS__ = {
    metadata: { currentWindow: { label: 'main' }, currentWebview: { windowLabel: 'main', label: 'main' } },
    transformCallback: () => cb++,
    unregisterCallback: () => {},
    convertFileSrc: (s) => s,
    invoke: async (cmd, a = {}) => {
      switch (cmd) {
        case 'get_settings': return settings;
        case 'list_books': return data.books;
        case 'book_detail': return { ...data.detail, position };
        case 'book_cover': return data.covers[a.id] ? b64(data.covers[a.id]) : new ArrayBuffer(0);
        case 'sentences': return data.sentences.filter((s) => s.idx >= a.from && s.idx < a.from + a.count);
        case 'lookup': return data.lookups[`${a.text}|${a.offset}`] ?? { entries: [], length: 0, has_dictionaries: true };
        case 'translate':
        case 'cached_translation': return data.translations[String(a.idx)] ?? null;
        case 'list_dictionaries': return data.dictionaries;
        case 'dictionary_catalog': return data.catalog;
        default: return null; // set_position, set_settings, plugin:event|listen, …
      }
    },
  };
}

const data = { ...fx, books, covers: coverById };
const base = { fontFamily: 'serif', fontSize: 30, furigana: true, translation: false, showBreakdown: true, showContext: false };
const desktop = { viewport: { width: 1180, height: 740 }, deviceScaleFactor: 2 };
const phone = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true };

async function shot(name, ctxOpts, settings, position, act) {
  const ctx = await browser.newContext({ ...ctxOpts, colorScheme: settings.theme === 'dark' ? 'dark' : 'light' });
  await ctx.addInitScript(bridge, { data, settings: { ...base, ...settings }, position });
  const page = await ctx.newPage();
  await page.goto(url);
  await page.waitForTimeout(400);
  await act(page);
  await page.mouse.move(1, 1); // no stray :hover highlight
  await page.waitForTimeout(700); // let transitions settle
  await page.screenshot({ path: join(outDir, `${name}.png`) });
  await ctx.close();
  console.log('✓', name);
}

const openBook = async (page) => {
  await page.locator('.cover-btn').first().click();
  await page.locator('.sentence').waitFor();
};
const clickChar = async (page, ch) => {
  const el = page.locator('.sentence [data-o]', { hasText: ch }).first();
  await el.click();
  await page.locator('.entry').first().waitFor();
};

await shot('library', desktop, { theme: 'light' }, 2, async () => {});
await shot('reader', desktop, { theme: 'dark' }, 2, openBook);
await shot('lookup', desktop, { theme: 'dark' }, 2, async (p) => { await openBook(p); await clickChar(p, '見'); });
await shot('translation', phone, { theme: 'dark', translation: true, fontSize: 24 }, 3, async (p) => {
  await openBook(p);
  await p.locator('.parts').waitFor();
});
await shot('mobile-lookup', phone, { theme: 'sepia', fontSize: 28 }, 0, async (p) => { await openBook(p); await clickChar(p, '猫'); });
await shot('options', phone, { theme: 'light', fontFamily: 'sans', fontSize: 26 }, 4, async (p) => {
  await openBook(p);
  await p.locator('.pill', { hasText: 'Aa' }).click();
});

await browser.close();
await server.close();
