# 0009. Reader interaction model and motion design

- Status: accepted
- Date: 2026-11-12

## Context

Reading one sentence at a time must feel instantaneous and calm. Taps on
words open lookups, so taps cannot also be used for page turning.

## Decision

- **Layout:** the sentence is centred both ways at a large size (user
  adjustable) in a Mincho or Gothic font. The top bar has back, chapter title,
  TOC, and the furigana and translation toggles. The bottom bar has
  previous/next buttons, a progress bar that can be scrubbed, and
  `n / total · %`. The translation panel slides up under the sentence.
- **Navigation:** horizontal swipe on touch, ← → / Space / PageUp/PageDown on
  desktop, and buttons everywhere. Tapping a word opens a lookup. Tapping
  empty space closes the popup.
- **Motion:** sentence change uses a 180 ms cross-slide of 24 px in the travel
  direction with opacity, and both elements are absolutely positioned so the
  layout never jumps. Popups use a fade plus 8 px rise, and sheets slide up.
  Progress bar width transitions in 200 ms. `prefers-reduced-motion` turns all
  of these into plain fades.
- **Speed:** sentences are fetched in windows of 30 (analyzed in Rust) and
  prefetched when within 8 of the window edge, so navigation never waits on
  IPC. Position is saved with a 400 ms debounce and on exit.
- **Furigana:** rendered with native `<ruby>` and toggled with a CSS class,
  so there is no re-render. The ruby row's height is always reserved, so
  toggling furigana does not shift the text.

## Consequences

- No tap-zones for paging. The swipe threshold (60 px, mostly horizontal) avoids
  conflicts with word taps.
