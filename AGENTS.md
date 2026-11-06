# Repository Guidelines

## Project Structure & Module Organization

- `src/`: React and TypeScript UI. Pages are in `src/pages/`, reusable UI in `src/components/`, Tauri wrappers in `src/services/`, and Zustand state in `src/store/`.
- `src-tauri/src/`: Rust code. Keep shared state/configuration in `core/`, LAN discovery and QUIC code in `network/`, transfer behavior in `transfer/`, persistence in `storage/`, and Tauri commands in `ipc/`.
- `src/assets/`: frontend assets; `src-tauri/icons/`: packaged application icons.
- `DESIGN.md`, `FEATURES.md`, and `README.md`: product and architecture references. Read `DESIGN.md` before changing UI.

## Build, Test, and Development Commands

Use `pnpm` (do not use npm directly):

```bash
pnpm install       # install JavaScript dependencies
pnpm dev           # run the Vite frontend at localhost:1420
pnpm tauri dev     # run the desktop app in development mode
pnpm build         # type-check and create the frontend production build
pnpm tauri build   # build distributable Tauri bundles for the host platform
pnpm preview       # serve the production frontend locally
```

For Rust checks, run `cargo check`, `cargo fmt --check`, and `cargo clippy --all-targets --all-features` from `src-tauri/`. There is no automated frontend test script or dedicated test suite; use these checks plus manual transfer/discovery verification.

## Coding Style & Naming Conventions

Use strict TypeScript, two-space indentation, and the existing project formatting. Name React components and pages in `PascalCase` (`DeviceList.tsx`), hooks/stores with `useX` (`useDeviceStore.ts`), and variables/functions in `camelCase`. Rust modules/files use `snake_case`; structs, enums, and traits use `PascalCase`. Reuse existing Ant Design, Tailwind, and Zustand patterns. Preserve cross-platform behavior with `PathBuf`, Tauri path APIs, and targeted `#[cfg(target_os = ...)]` blocks.

## Testing Guidelines

There is no Jest/Vitest suite or coverage threshold. Every change should pass `pnpm build`; Rust changes should also pass the checks above. For network, filesystem, or platform-specific changes, test on Windows and macOS when possible, including discovery, send/pause/resume/cancel, and download path handling.

## Commit & Pull Request Guidelines

Follow the existing Conventional Commit style: `feat:`, `fix:`, `docs:`, `refactor:`, `perf:`, `test:`, or `chore:` plus a concise imperative summary. Keep commits focused. Pull requests should explain the change, list verification commands, link issues, and include screenshots or recordings for UI changes. Call out platform-specific behavior and changes to Tauri capabilities, ports, or file permissions.

## Security & Configuration Tips

Do not commit secrets, local settings, build output, or `src-tauri/target`. Review `src-tauri/capabilities/default.json` and `tauri.conf.json` when adding IPC, filesystem, shell, or notification behavior. Preserve UDP discovery port `45678` and QUIC port `45679` unless protocol documentation changes too.
