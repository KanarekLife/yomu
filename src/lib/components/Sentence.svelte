<script lang="ts">
  import type { SentenceView } from '../types';

  let {
    sentence,
    furigana,
    highlight = null,
    soft = null,
    onpick,
  }: {
    sentence: SentenceView;
    furigana: boolean;
    highlight?: { start: number; len: number } | null;
    soft?: { start: number; len: number } | null;
    onpick?: (offset: number, el: HTMLElement) => void;
  } = $props();

  type C = { c: string; o: number };
  type S = { ruby?: string; chars: C[] };
  type T = { punct: boolean; segs: S[] };

  const layout = $derived.by<T[]>(() => {
    return sentence.tokens.map((t) => {
      let o = t.start;
      return {
        punct: t.punct,
        segs: t.segs.map((s) => ({ ruby: s.ruby, chars: Array.from(s.text).map((c) => ({ c, o: o++ })) })),
      };
    });
  });

  const inRange = (o: number, r: { start: number; len: number } | null) => !!r && o >= r.start && o < r.start + r.len;

  function click(e: MouseEvent) {
    const el = (e.target as HTMLElement).closest<HTMLElement>('[data-o]');
    if (!el) return;
    e.stopPropagation();
    onpick?.(Number(el.dataset.o), el);
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<p class="sentence" class:nofuri={!furigana} lang="ja" onclick={click}>
  {#each layout as t}<span class="tok" class:punct={t.punct}
      >{#each t.segs as s}{#if s.ruby}<ruby
            >{#each s.chars as ch}<span data-o={ch.o} class:hl={inRange(ch.o, highlight)} class:soft={inRange(ch.o, soft)}>{ch.c}</span
              >{/each}<rt>{s.ruby}</rt></ruby
          >{:else}{#each s.chars as ch}<span data-o={ch.o} class:hl={inRange(ch.o, highlight)} class:soft={inRange(ch.o, soft)}>{ch.c}</span
            >{/each}{/if}{/each}</span
    >{/each}
</p>

<style>
  .sentence {
    margin: 0;
    line-height: 2.1;
    text-align: start;
    word-break: normal;
    line-break: strict;
    overflow-wrap: anywhere;
    cursor: text;
  }
  .tok { border-radius: 4px; transition: background 120ms; }
  .tok:not(.punct):hover { background: var(--accent-soft); cursor: pointer; }
  ruby { ruby-position: over; ruby-align: center; }
  rt {
    font-size: 0.45em;
    color: var(--muted);
    user-select: none;
    transition: opacity 160ms var(--ease);
  }
  /* display:none (not opacity) so hidden furigana doesn't widen the kanji it sits on. */
  .nofuri rt { display: none; }
  [data-o] { border-radius: 3px; transition: background 140ms var(--ease), box-shadow 140ms var(--ease); }
  .hl { background: var(--hl); box-shadow: 0 2px 0 var(--accent); }
  .soft { background: var(--accent-soft); }
</style>
