<script lang="ts">
  // Minimal inline icon set (stroke icons, 24px grid).
  let { name, size = 22 }: { name: string; size?: number } = $props();
  const paths: Record<string, string> = {
    back: 'M15 18l-6-6 6-6',
    prev: 'M15 18l-6-6 6-6',
    next: 'M9 18l6-6-6-6',
    plus: 'M12 5v14M5 12h14',
    settings:
      'M12 15a3 3 0 100-6 3 3 0 000 6zM19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 11-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 11-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 110-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 114 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 112.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 110 4h-.09a1.65 1.65 0 00-1.51 1z',
    toc: 'M4 6h16M4 12h16M4 18h10',
    trash: 'M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6',
    close: 'M18 6L6 18M6 6l12 12',
    refresh: 'M21 12a9 9 0 11-3-6.7L21 8M21 3v5h-5',
    up: 'M18 15l-6-6-6 6',
    down: 'M6 9l6 6 6-6',
    book: 'M4 19.5A2.5 2.5 0 016.5 17H20V3H6.5A2.5 2.5 0 004 5.5v14zM20 17v4H6.5A2.5 2.5 0 014 18.5',
    more: 'M12 13a1 1 0 100-2 1 1 0 000 2zM19 13a1 1 0 100-2 1 1 0 000 2zM5 13a1 1 0 100-2 1 1 0 000 2z',
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"><path d={paths[name] ?? ''} /></svg>
