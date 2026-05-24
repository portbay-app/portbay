# Contributing to PortBay

PortBay is a native local-development manager built with Tauri 2, Rust, and Svelte 5. Contributions should be small, traceable, and tied to an accepted issue.

## Local Setup

```bash
pnpm install
./scripts/fetch-caddy.sh
./scripts/fetch-mkcert.sh
./scripts/fetch-mailpit.sh
./scripts/fetch-cloudflared.sh
cd src-tauri && cargo test
cd .. && pnpm check
pnpm tauri dev
```

Sidecar binaries are stored under `src-tauri/binaries/` and are ignored by git.

## Workflow

- Use one branch per issue.
- Keep commits atomic and conventional, for example `feat(hosts): add helper client`.
- Link the issue or kanban card in the pull request.
- Keep unrelated refactors out of feature commits.
- Do not commit generated sidecar binaries, local registries, credentials, crash reports, or build output.

## Quality Bar

Before opening a pull request:

```bash
cd src-tauri && cargo test
cd .. && pnpm check
pnpm build
```

Run narrower checks while iterating, but the full set should pass before review.

## Architecture Expectations

- Prefer existing Rust modules and Svelte stores over new abstractions.
- Keep Tauri commands behind `src-tauri/src/commands/`.
- Return structured `AppError` values instead of string errors.
- Keep filesystem writes atomic where user data is involved.
- Treat project paths, environment variables, logs, and registry contents as private user data.

## Pull Request Checklist

- Tests added or updated for backend behavior.
- `pnpm check` passes for frontend changes.
- Docs updated for user-visible behavior.
- New Tauri capabilities are justified and scoped.
- The PR is one coherent change, not a bundle of unrelated work.

## Questions

Use GitHub Discussions for setup questions and design discussion. Use issues for reproducible bugs and accepted feature work.
