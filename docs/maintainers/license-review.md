# License Review Guide

**This is not legal advice.** This guide describes the practices PortBay maintainers follow to assess license compatibility for incoming code and dependencies. For questions with meaningful legal or financial consequences, consult a qualified attorney.

---

## 1. Project license baseline

PortBay is licensed under **AGPL-3.0-only**. This means:

- All first-party source files must carry the `AGPL-3.0-only` SPDX identifier.
- Any code added to this repo — whether written from scratch, lifted, or adapted — must be compatible with AGPL-3.0-only.
- The copyleft extends to network use: anyone who runs a modified version of PortBay as a network service must publish their modifications.

**SPDX header convention** for first-party files:

```
// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) 2026 Tribal House
```

Adjust the comment syntax for the file type (e.g., `#` for shell/TOML, `<!--` for HTML/Svelte templates).

---

## 2. Inbound license compatibility

### 2.1 Compatible without concern

These licenses are permissive and compatible with AGPL-3.0:

| License | Notes |
|---|---|
| MIT | Permissive. Compatible. Attribution required in NOTICE. |
| BSD-2-Clause | Permissive. Compatible. Attribution required in NOTICE. |
| BSD-3-Clause | Permissive. Compatible. Attribution required in NOTICE. |
| ISC | Permissive. Equivalent to MIT in effect. Attribution required in NOTICE. |
| Apache-2.0 | Permissive. Compatible with GPLv3/AGPL-3.0 per FSF guidance. Attribution required in NOTICE. |
| CC0-1.0 / public domain | No restrictions. No attribution required (though still courteous to note). |

"Attribution required in NOTICE" means a NOTICE entry must be added following the pattern in the top-level `NOTICE` file before merge.

### 2.2 Incompatible or needs-care

| License | Status | Guidance |
|---|---|---|
| GPL-2.0-only | Incompatible | GPL-2.0 is not compatible with AGPL-3.0. Reject. |
| GPL-2.0-or-later | Potentially compatible | Can be treated as GPL-3.0-or-later, which is compatible with AGPL-3.0. Verify the specific crate/package explicitly says "or later." |
| GPL-3.0-only / GPL-3.0-or-later | Compatible | But copyleft strengthens. Note in review. |
| LGPL-2.1 / LGPL-3.0 | Evaluate carefully | LGPL can be compatible in specific use patterns. Discuss with the team before accepting. |
| MPL-2.0 | File-level copyleft | Compatible with AGPL-3.0 per FSF. Acceptable, but MPL-licensed files must remain under MPL. |
| AGPL-3.0 (other project) | Compatible | Already AGPL. Note in NOTICE. |
| SSPL / BSL / proprietary | Incompatible | Reject. These are not open-source licenses under OSI definition. |
| CC-BY-NC-* / CC-BY-ND-* | Incompatible | Non-commercial and no-derivatives clauses are incompatible with open-source distribution. |
| No license stated | Reject by default | Code without a license is all-rights-reserved by default. Request the contributor add an explicit license or use code with a clear license instead. |

When in doubt, the answer is "no" until a maintainer with more context says otherwise.

---

## 3. Checking a new dependency's license

### 3.1 Rust (Cargo)

Every new `Cargo.toml` dependency must have its license verified before merging.

**Quick check:**

```sh
# View a specific crate's license field
cargo metadata --no-deps --format-version 1 | jq '.packages[] | select(.name=="<crate>") | {name, license}'
```

**Bulk audit with cargo-deny:**

```sh
cargo install cargo-deny
cargo deny check licenses
```

`cargo-deny` is configured via `deny.toml` (or `Cargo.deny.toml`). The allowed license list in that file should match section 2 of this document. If `deny.toml` does not yet exist in the repo, creating one is a worthwhile contribution.

**What to look for:**
- The crate's `license` field in `Cargo.toml` (SPDX expression).
- If the field is absent or `license-file` is used instead, read the referenced file.
- Check crates.io for the crate's declared license if the local metadata is unclear.

### 3.2 JavaScript/TypeScript (npm/pnpm)

Every new entry in `package.json` dependencies must be checked.

**Quick check:**

```sh
# For a specific package
cat node_modules/<package>/package.json | jq '{name, license}'

# Bulk audit
npx license-checker --summary --production
```

**What to look for:**
- The `license` field in `package.json`.
- If absent, look for `LICENSE`, `LICENCE`, or `COPYING` in the package root.
- Some packages declare `UNLICENSED` — these should be treated as proprietary and rejected unless the maintainer has explicit permission.

**Dev dependencies** (not shipped in the final binary) have relaxed requirements but should still avoid GPL-incompatible licenses for consistency and to avoid tool-chain complications.

---

## 4. Lifted or adapted third-party code

"Lifted" means copying code from an external project directly into this repo (rather than taking it as a dependency). This requires more diligence than adding a package.

**Requirements before merging lifted code:**

1. **License is compatible** (see section 2).
2. **A NOTICE entry exists** in the top-level `NOTICE` file following the established pattern. The entry must include:
   - Upstream project name and URL
   - Copyright holder(s)
   - License
   - Which PortBay files correspond to which upstream files
   - A brief note on what was adapted or changed

   See the existing Lerd MIT entry in `NOTICE` as the canonical example.

3. **In-file comment** pointing back to the upstream path (the Lerd entries in `NOTICE` describe this convention: a comment on the first line of the file or near the top referencing the upstream source).

4. **DCO sign-off** from the contributor certifying they have the right to contribute this adaptation.

**Never lift code:**
- From a project with no stated license.
- From a project with an incompatible license (see section 2.2).
- From a contributor who cannot establish they have the right to relicense it (e.g., they did not write it and it is not clearly open-source).

---

## 5. Code provenance and the DCO

The Developer Certificate of Origin (DCO) sign-off (`Signed-off-by:` trailer, added via `git commit -s`) is the contributor's certification that they wrote the code or have the right to contribute it.

DCO does **not** substitute for license compatibility review — it is a certification of rights, not a license grant. Both must be verified.

**Provenance questions to ask when something looks lifted:**

- "Can you point to the source you adapted this from?"
- "Is that project's license compatible with AGPL-3.0?"
- "Have you added a NOTICE entry?"

If a contributor cannot answer these questions clearly, do not merge. It is better to lose a contribution than to accept code of uncertain provenance.

**If you discover post-merge that code was lifted without proper attribution:** treat it as a priority issue. Add the NOTICE entry, verify license compatibility, and if the license turns out to be incompatible, remove the code in the next patch.

---

## 6. Future separately-licensed packages

If a PortBay utility or library is extracted into a standalone package that has no AGPL obligations (e.g., a generic host-parsing library with no PortBay-specific logic), it may be appropriate to license that package under MIT or Apache-2.0 rather than AGPL-3.0.

This is planned but has not happened yet. When it does:

- The package must live in its own repository or a clearly separated workspace package with its own `Cargo.toml` / `package.json`.
- The license choice must be explicitly documented in that package's `README` and `LICENSE` file.
- The boundary between the MIT/Apache-licensed package and the AGPL-licensed PortBay application must be clean — the package must not import PortBay-internal APIs or types.
- This decision must be made explicitly by the maintainer team, not assumed by contributors.

---

## 7. Quick reference: decision tree for incoming code

```
Is the code's license stated clearly?
  No → Ask for a license. Reject if none can be established.
  Yes ↓

Is the license in the "compatible" list (section 2.1)?
  No → Is it in the "needs-care" list (section 2.2)?
    Yes → Review carefully; discuss with team.
    No → Reject.
  Yes ↓

Is the code lifted/adapted (not a package dependency)?
  Yes → Require NOTICE entry and in-file comment before merge.
  No ↓

Is it a Rust dependency? → Run cargo-deny check licenses.
Is it a JS dependency? → Run license-checker.

Does the audit pass? → Approve (license aspect). Continue with other review gates.
```

---

**This is not legal advice.** Maintainers apply these guidelines in good faith. When a situation is ambiguous or the stakes are high, get qualified legal counsel before proceeding.
