<script lang="ts">
  import { morae } from '../glossary';
  let { reading, position }: { reading: string; position: number } = $props();
  const m = $derived(morae(reading));
  const high = (i: number) =>
    position === 0 ? i > 0 : position === 1 ? i === 0 : i > 0 && i < position;
</script>

<span class="pitch" title="[{position}]">
  {#each m as mora, i}
    <span class="mora" class:h={high(i)} class:rise={high(i) && i > 0 && !high(i - 1)} class:drop={position > 0 && i === position - 1}>{mora}</span>
  {/each}
  <span class="num">{position}</span>
</span>

<style>
  .pitch { display: inline-flex; align-items: flex-end; font-size: 16px; line-height: 1.2; padding-top: 3px; }
  .mora { border-top: 1.5px solid transparent; padding: 0 1px; }
  .h { border-top-color: currentColor; }
  .rise { border-left: 1.5px solid currentColor; }
  .drop { border-right: 1.5px solid currentColor; }
  .num { font-size: 11px; margin-left: 4px; color: var(--muted); }
</style>
