# 0003. Svelte 5 + Vite + TypeScript for the UI

- Status: accepted
- Date: 2026-11-12

## Context

The UI must be clean, responsive and quick, with tasteful animations, on both
phones and desktops. The WebView is the rendering engine (ADR-0002).

## Decision

- **Svelte 5** (runes) compiled by **Vite**, in **TypeScript**. Svelte compiles
  to minimal DOM updates with no virtual DOM, which suits low-end Android devices.
- No UI component library. A small hand-written design system in
  `src/app.css` (CSS custom properties for themes: light, dark, sepia,
  following the system by default).
- Animations use Svelte transitions and CSS only, animating `transform` and
  `opacity` (compositor-friendly). Durations are 120–260 ms with standard
  easing, and all of them respect `prefers-reduced-motion`.
- Navigation between Library, Reader and Settings is a small state machine in
  `src/lib/state.svelte.ts` (no router dependency).

## Consequences

- Small bundle; fast cold start in the WebView.
- We own the styling. Consistency depends on the shared tokens in `app.css`.
