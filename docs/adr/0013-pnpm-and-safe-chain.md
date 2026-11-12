# 0013. pnpm with Aikido Safe Chain for JavaScript dependencies

- Status: accepted
- Date: 2026-11-12

## Context

The UI depends on an npm toolchain (Vite, Svelte, the Tauri CLI, Playwright for screenshots).
npm-ecosystem supply-chain attacks are frequent, and most of them are caught within a day
or two of publication. We also want fast, deterministic installs.

## Decision

- Use **pnpm** (version pinned via `packageManager` in `package.json`, lockfile
  `pnpm-lock.yaml`) for all JavaScript installs and scripts. CI uses
  `pnpm install --frozen-lockfile`.
- Route every install through **[Aikido Safe Chain](https://github.com/AikidoSec/safe-chain)**:
  - locally, its shell integration wraps `pnpm` and `pnpx`;
  - in CI, the release workflow installs it with `--ci`, which puts PATH shims in front of `pnpm`, on Linux and Windows;
  - in the podman build images, it is installed in `/opt/safe-chain`.
  - The installer is pinned to a release and verified by SHA-256 everywhere.
- Respect Safe Chain's **48-hour minimum package age**. When a just-released version
  is held back, lower the requirement instead of passing `--safe-chain-skip-minimum-package-age`.
  For example, Vite is pinned to `^8.3.7` because 8.3.8 was 36 hours old at adoption.

## Consequences

- Malicious packages and very fresh releases cannot enter through `pnpm install`.
- Upgrades lag new releases by at least two days.
- Contributors need Safe Chain installed (see the README) for the same protection locally.
