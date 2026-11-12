// Safe renderer for Yomitan glossaries, including structured content
// (ADR-0007). Builds DOM nodes from a whitelist; never uses innerHTML.

const TAGS = new Set([
  'span', 'div', 'ol', 'ul', 'li', 'table', 'thead', 'tbody', 'tfoot', 'tr', 'td', 'th',
  'ruby', 'rt', 'rp', 'br', 'details', 'summary', 'sub', 'sup', 'b', 'i', 'em', 'strong', 'a',
]);
const STYLE_KEYS = [
  'fontStyle', 'fontWeight', 'fontSize', 'textDecorationLine', 'verticalAlign', 'textAlign',
  'marginTop', 'marginBottom', 'marginLeft', 'marginRight', 'paddingLeft', 'listStyleType', 'whiteSpace',
] as const;

type Obj = Record<string, unknown>;

function renderSC(c: unknown, parent: Node, depth = 0): void {
  if (depth > 64 || c == null) return;
  if (typeof c === 'string') {
    parent.appendChild(document.createTextNode(c));
    return;
  }
  if (Array.isArray(c)) {
    for (const x of c) renderSC(x, parent, depth + 1);
    return;
  }
  if (typeof c !== 'object') return;
  const o = c as Obj;
  const tag = typeof o.tag === 'string' ? o.tag : 'span';
  if (tag === 'img' || tag === 'image') return;
  const el = document.createElement(TAGS.has(tag) ? (tag === 'a' ? 'span' : tag) : 'span');
  if (tag === 'a') el.className = 'sc-link';
  if (o.style && typeof o.style === 'object') {
    const s = o.style as Obj;
    for (const k of STYLE_KEYS) if (typeof s[k] === 'string') (el.style as unknown as Obj)[k] = s[k];
  }
  if (o.data && typeof o.data === 'object') {
    for (const [k, v] of Object.entries(o.data as Obj)) {
      // setAttribute, not el.dataset: dataset throws on names like "sentence-key".
      if (/^[a-zA-Z0-9-]+$/.test(k) && typeof v === 'string') el.setAttribute(`data-sc-${k.toLowerCase()}`, v);
    }
  }
  if (typeof o.title === 'string') el.title = o.title;
  if (typeof o.lang === 'string') el.lang = o.lang;
  if (tag === 'td' || tag === 'th') {
    if (typeof o.colSpan === 'number') (el as HTMLTableCellElement).colSpan = o.colSpan;
    if (typeof o.rowSpan === 'number') (el as HTMLTableCellElement).rowSpan = o.rowSpan;
  }
  renderSC(o.content, el, depth + 1);
  parent.appendChild(el);
}

/** Render one glossary item into a fresh element. */
function renderItem(item: unknown): HTMLElement {
  const el = document.createElement('div');
  el.className = 'gloss-item';
  if (typeof item === 'string') el.textContent = item;
  else if (item && typeof item === 'object') {
    const o = item as Obj;
    if (o.type === 'text' && typeof o.text === 'string') el.textContent = o.text;
    else if (o.type === 'structured-content') renderSC(o.content, el);
    else if (Array.isArray(item)) el.textContent = item.filter((x) => typeof x === 'string').join(' / ');
  }
  return el;
}

function render(node: HTMLElement, glossary: unknown) {
  try {
    renderUnsafe(node, glossary);
  } catch (e) {
    // Never lose a definition to a renderer bug: fall back to plain text.
    node.replaceChildren();
    node.textContent = plainText(glossary);
    console.error('glossary render failed', e);
  }
}

function plainText(v: unknown): string {
  if (typeof v === 'string') return v;
  if (Array.isArray(v)) return v.map(plainText).filter(Boolean).join('; ');
  if (v && typeof v === 'object') {
    const o = v as Obj;
    if (o.tag === 'rt' || o.tag === 'img') return '';
    return plainText(o.text ?? o.content);
  }
  return '';
}

function renderUnsafe(node: HTMLElement, glossary: unknown) {
  node.replaceChildren();
  const items = Array.isArray(glossary) ? glossary : [glossary];
  if (items.length === 1) {
    node.appendChild(renderItem(items[0]));
    return;
  }
  const ol = document.createElement('ol');
  for (const it of items) {
    const li = document.createElement('li');
    li.appendChild(renderItem(it));
    ol.appendChild(li);
  }
  node.appendChild(ol);
}

/** Svelte action: `<div use:glossary={definition.glossary}>`. */
export function glossary(node: HTMLElement, value: unknown) {
  render(node, value);
  return { update: (v: unknown) => render(node, v) };
}

const SMALL = new Set('ゃゅょぁぃぅぇぉゎャュョァィゥェォヮ'.split(''));
export function morae(reading: string): string[] {
  const out: string[] = [];
  for (const c of reading) {
    if (SMALL.has(c) && out.length) out[out.length - 1] += c;
    else out.push(c);
  }
  return out;
}

export function charsOf(s: string): string[] {
  return Array.from(s);
}
