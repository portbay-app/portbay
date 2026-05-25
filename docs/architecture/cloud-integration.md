# Cloud Integration

This document explains how PortBay Community integrates with PortBay Cloud
without pulling any proprietary code into the public repository. It is the
design companion to [repo boundaries](./repo-boundaries.md).

## Principles

1. **Local-first, offline-capable.** The Community app is fully usable with no
   account and no network. Cloud features are additive; nothing local is
   crippled to upsell Pro. If PortBay Cloud is unreachable, the app keeps
   working with its last known state.
2. **Cloud/Pro is optional.** Sign-in, sync, and entitlements are opt-in. A user
   who never signs in never talks to PortBay Cloud (beyond, if enabled, opt-in
   telemetry).
3. **Talk through stable public APIs.** All communication is over documented,
   versioned, public HTTP APIs. The app depends on the *contract*, not on the
   server's internals.
4. **The public client stays thin and generic.** It knows a base URL, request
   and response shapes, and how to verify a signature. It contains no business
   logic, no secrets, and no private endpoints.
5. **Trust flows from a public key, not shared secrets.** Entitlements are
   signed by PortBay Cloud and verified client-side with an embedded **public**
   key. The signing **private** key exists only in `portbay-cloud`.

## Where the client boundary already lives

We deliberately did **not** create a separate `packages/cloud-client/` crate.
The cloud client already exists as a thin, well-bounded layer inside the Rust
core, and a standalone package would add structure without adding isolation. The
relevant modules are:

| Module | Role | What it must never do |
|---|---|---|
| `src-tauri/src/auth/` | Login handshake, session storage in the OS keychain, token refresh against the public API. | Implement the auth server, store secrets in plaintext, or embed private endpoints. |
| `src-tauri/src/entitlements/` | Verify a signed entitlement with the embedded **public** key; cache it; honor offline grace. | Sign, issue, or validate entitlements server-side; embed the private key. |
| `src-tauri/src/commands/sync.rs` | Encrypt the registry locally, push/pull opaque ciphertext, manage devices. | Decrypt on the server, or send plaintext user data. |
| `src-tauri/src/commands/{auth,entitlements,telemetry}.rs` | The IPC surface the UI calls. | Carry business logic that belongs server-side. |

These are the files the CODEOWNERS "cloud boundary" rules cover, and the ones
reviewers scrutinize most.

## The contract

The public contract (entitlement tiers, the signed-license wire format, grace
windows, and offline behavior) is documented in
[`docs/pro/entitlements.md`](../pro/entitlements.md), which is intentionally
public: the shipped app reads that format, so the server cannot change it
unilaterally without breaking client verification. Publishing the contract does
not expose the implementation — it pins it.

The **server-side** design (how the backend is built and deployed, its data
model, its security review) lives in the private `portbay-cloud` repository, not
here. If you're looking for it in this repo, that's the boundary working as
intended.

## Entitlement verification, in brief

```
PortBay Cloud (private)                 PortBay Community (this repo)
─────────────────────────              ───────────────────────────────
signs entitlement with                  fetches signed entitlement over
LICENSE_SIGNING_KEY (private)  ──────▶   the public API, then verifies it
                                         with the embedded PUBLIC key,
                                         caches it, and trusts it offline
                                         within a grace window.
```

The client never sees the private key and never makes an entitlement decision
the server didn't sign. A user can read every line of this verification in the
public source — that transparency is the point of an AGPL client.

## Adding a new cloud-backed feature

When you build a feature that needs PortBay Cloud:

1. Define the **public** request/response shapes and add them to the contract
   docs.
2. Put only the **client** half here — a generic call plus local handling
   (encrypt before upload, verify after download).
3. Put the **server** half, and anything secret or proprietary, in
   `portbay-cloud`.
4. Run `scripts/check-repo-boundaries.sh` before you push.

If you find yourself wanting to add a secret, a private endpoint, or business
logic to this repo to make the feature work, that's the signal it belongs on the
other side of the boundary.
