# Early-access feature flags

PortBay Pro includes **early access**: Pro accounts can opt into in-development
features before they reach the stable channel. This page is the convention for
authors putting a feature behind an early-access flag and later graduating it.

## How it works

A feature has a rollout **stage**:

- `ga` — generally available. On for everyone. (The default for any unregistered
  feature id.)
- `early` — on **only** for an account with the `early_access` entitlement (Pro)
  that has turned on **Settings → Early Access**.

Resolution lives in two mirrored places that must stay in lockstep:

| Layer | File | API |
|---|---|---|
| Client (Svelte) | `src/lib/stores/flags.svelte.ts` | `flags.enabled("id")` |
| Core (Rust) | `src-tauri/src/flags.rs` | `flags::enabled("id", &ent, opted_in)` |

The opt-in is the `earlyAccessOptIn` preference
(`preferences.json` → `early_access_opt_in`), surfaced as a Pro-gated toggle in
`EarlyAccessSection.svelte`.

## Putting a feature behind early access

1. **Register the id** in *both* registries with stage `early`:

   ```ts
   // src/lib/stores/flags.svelte.ts
   const REGISTRY: Record<string, Stage> = {
     "experimental-tunnels": "early",
   };
   ```

   ```rust
   // src-tauri/src/flags.rs — fn stage()
   match feature {
       "experimental-tunnels" => Stage::Early,
       _ => Stage::Ga,
   }
   ```

2. **Gate at the call site** — never branch on the entitlement directly:

   ```svelte
   {#if flags.enabled("experimental-tunnels")}
     <ExperimentalTunnels />
   {/if}
   ```

   ```rust
   if flags::enabled("experimental-tunnels", &ent, opted_in) { /* … */ }
   ```

3. **Graduate to stable** — flip the stage to `ga` (or remove the entry). No
   change is needed at any call site; everyone gets it on the next release.

## Scope notes

- Source of truth is **ship-time** (the tables above). Backend-driven overrides
  (toggling a flag without a release) are intentionally deferred — see the
  kanban card. Don't promise remote toggling in the matrix/marketing until it
  exists.
- The flag system is **not** a security boundary. Like every Pro gate it's an
  honest limit, bypassable by rebuilding (see [`entitlements.md`](./entitlements.md)
  §1). Don't put anything behind a flag that must not run for a determined user.
