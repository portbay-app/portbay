# Third-party models & AI assets

This manifest lists every model, weight, and AI-adjacent asset PortBay ships, downloads on demand, or pulls, with its license and the clearance decision made for it. It is **generated** — do not edit by hand.

- **Generated:** 2026-07-17
- **Register (source of truth):** [`docs/compliance/model-license-register.json`](./docs/compliance/model-license-register.json) (COMP-5)
- **Catalog (in-app licenses):** [`src-tauri/resources/default-model-catalog.json`](./src-tauri/resources/default-model-catalog.json)
- **Generator:** `scripts/gen-model-license-manifest.mjs` (COMP-2)
- **Totals:** 56 assets — 34 in-catalog, 22 off-catalog, 6 excluded (not shipped), 0 un-cleared.

Adjudication values: `clear` (permissive, ship freely) · `clear-with-condition` (attribution / use-restriction pass-through) · `user-loaded-only` (user supplies the weights) · `accepted-risk-with-rationale` (owner-accepted residual risk) · `excluded` (deliberately not shipped).

## Shipping catalog — speech-to-text

| Model | Source | License (in-app) | Distribution | Adjudication |
|---|---|---|---|---|
| Parakeet EOU 120M (streaming) | FluidInference (Parakeet acoustic end-of-utterance) | CC-BY-4.0 | user-downloaded | `clear-with-condition` |
| Parakeet TDT v3 (0.6B) | NVIDIA parakeet-tdt-0.6b-v3 | CC-BY-4.0 | user-downloaded | `clear-with-condition` |
| Parakeet TDT v2 (0.6B, English) | NVIDIA parakeet-tdt-0.6b-v2 | CC-BY-4.0 | user-downloaded | `clear-with-condition` |
| Nemotron Speech 3.5 (English, streaming) | NVIDIA Nemotron, staged via FluidInference | CC-BY-4.0 | user-downloaded | `clear-with-condition` |
| Qwen3 ASR (0.6B) | Qwen (Alibaba), staged via FluidInference | Apache-2.0 | user-downloaded | `clear` |
| Whisper Large v3 Turbo | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Distil-Whisper Large v3 | OpenAI Whisper (distilled) | MIT | user-downloaded | `clear` |
| Whisper Large v3 | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Whisper Medium (English) | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Whisper Small | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Whisper Base | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Whisper Tiny | OpenAI Whisper | MIT | user-downloaded | `clear` |
| Whisper Tiny (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |
| Whisper Base (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |
| Whisper Small (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |
| Whisper Medium (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |
| Whisper Large v3 Turbo (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |
| Whisper Large v3 (GGML) | whisper.cpp / ggml (ggerganov) | MIT | user-downloaded | `clear` |

## Shipping catalog — text-to-speech

| Model | Source | License (in-app) | Distribution | Adjudication |
|---|---|---|---|---|
| Chatterbox Turbo (English) | Resemble AI Chatterbox (mlx-community fp16) | MIT | user-downloaded | `clear` |
| Kokoro (English) | hexgrad Kokoro-82M | Apache-2.0 | user-downloaded | `clear` |
| Piper — Lessac (English, US, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Amy (English, US, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Ryan (English, US, high) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Lessac (English, US, high) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Alba (English, GB, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Northern English Male (English, GB, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Thorsten (German, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — Siwis (French, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |
| Piper — DaveFX (Spanish, medium) | rhasspy Piper voices | Varies by voice — see VOICES.md | user-downloaded | `user-loaded-only` |

## Shipping catalog — image generation

| Model | Source | License (in-app) | Distribution | Adjudication |
|---|---|---|---|---|
| Stable Diffusion 2.1 (base) | Apple Core ML SD (Stability AI weights) | CreativeML Open RAIL++-M | user-downloaded | `clear-with-condition` |
| FLUX.1 schnell · 4-bit | Black Forest Labs FLUX.1-schnell (mflux 4-bit) | Apache-2.0 | user-downloaded | `clear` |
| Stable Diffusion 1.5 | Apple Core ML SD (Stability AI weights) | CreativeML OpenRAIL-M | user-downloaded | `clear-with-condition` |
| SD-Turbo | Stability AI SD-Turbo (Core ML community layout) | Stability AI Community | user-downloaded | `clear-with-condition` |
| Stable Diffusion XL (base) | Apple Core ML SD (Stability AI weights) | CreativeML Open RAIL++-M | user-downloaded | `clear-with-condition` |

## Off-catalog assets (bundled VAD / wake / presence, Ollama, and excluded models)

| Model | Source | License (in-app) | Distribution | Adjudication |
|---|---|---|---|---|
| Silero VAD (silero_vad.onnx) | snakers4/silero-vad | MIT | bundled | `clear` |
| Smart Turn v3 (smart-turn-v3.onnx) | Daily / Pipecat smart-turn | BSD-2-Clause | bundled | `clear` |
| openWakeWord melspectrogram front-end (melspectrogram.onnx) | openWakeWord (dscripka) | Apache-2.0 | bundled | `clear` |
| Google speech_embedding extractor (embedding_model.onnx) | Google speech_embedding/1 (TFHub), converted by openWakeWord | Apache-2.0 | bundled | `clear` |
| "Hey PortBay" wake classifier (hey_portbay.onnx) | PortBay (first-party, owner-trained) | AGPL-3.0-only (first-party) | bundled | `clear` |
| orb-ui presence shaders (radial + cloud themes) | orb-ui (alexanderqchen) | MIT | bundled | `clear` |
| Presence pet — Porty (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| Presence pet — Moorly (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| Presence pet — Finder (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `accepted-risk-with-rationale` |
| Presence pet — Dot (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| Presence pet — Macintosh (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `accepted-risk-with-rationale` |
| Presence pet — Cubit (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| Presence pet — Yee (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `accepted-risk-with-rationale` |
| Presence pet — Nox (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| Presence pet — Nim (spritesheet + preview) | PortBay-original art (owner) | AGPL-3.0-only (first-party art) | bundled | `clear` |
| openWakeWord pre-trained wake classifiers (hey_jarvis, alexa, hey_mycroft, …) | openWakeWord (dscripka) | CC-BY-NC-SA-4.0 | bundled | `excluded` |
| TEN-VAD | TEN Framework | TEN-VAD license (on-device self-hosting restricted) | user-downloaded | `excluded` |
| Cohere Transcribe 03-2026 CoreML (formerly catalog id `cohere-transcribe`) — REMOVED from catalog 2026-07-17 | FluidInference/cohere-transcribe-03-2026-coreml (conversion of CohereLabs/cohere-transcribe-03-2026) | CONFLICTED at artifact level: repo YAML tag `apache-2.0` vs repo README License section "CC-BY-NC-4.0" | user-downloaded | `excluded` |
| LiveKit turn-detector weights | LiveKit Agents | LiveKit model license (no standalone use / no distillation) | user-downloaded | `excluded` |
| Porcupine wake engine (Picovoice) | Picovoice | Proprietary (Picovoice commercial) | user-downloaded | `excluded` |
| xLAM (Salesforce function-calling LLM) | Salesforce xLAM | CC-BY-NC-4.0 (NonCommercial) | user-downloaded | `excluded` |
| Ollama-pulled local LLMs (user-selected) | Ollama registry (ollama.com/library) | Varies per model (each under its own license) | Ollama-pulled | `user-loaded-only` |

## Conditions, exclusions & un-cleared notes

- **Parakeet EOU 120M (streaming)** (`clear-with-condition`): Clean acoustic-EOU streaming path (contrast: LiveKit turn-detector, excluded). CC-BY-4.0 condition = attribution, satisfied by the in-app catalog license disclosure + NOTICE.
- **Parakeet TDT v3 (0.6B)** (`clear-with-condition`): CC-BY-4.0 condition = attribution (in-app catalog + NOTICE).
- **Parakeet TDT v2 (0.6B, English)** (`clear-with-condition`): CC-BY-4.0 condition = attribution (in-app catalog + NOTICE).
- **Nemotron Speech 3.5 (English, streaming)** (`clear-with-condition`): CC-BY-4.0 condition = attribution (in-app catalog + NOTICE).
- **Piper — Lessac (English, US, medium)** (`user-loaded-only`): PortBay bundles/fetches NO Piper voice weights — verified 2026-07-17 in src-tauri/src/commands/tts.rs ('Nothing here auto-downloads'; piper_download_model returns 'PortBay doesn't download Piper voices yet') and every Piper catalog speedNote says 'manual install'. Per-voice license varies and a subset of rhasspy voices is NON-COMMERCIAL; rhasspy's VOICES.md does not enumerate per-voice licenses (verified 2026-07-17 — it is download links only), so per-voice terms live in each voice's dataset attribution. HARD GUARD: if any future change adds a sidecar/UI auto-download path for Piper voices, the NC voices MUST be pruned from the catalog and each remaining voice's license individually verified and recorded here FIRST — until then the adjudication user-loaded-only holds and PortBay bears no redistribution obligation. This guard applies to all nine Piper rows below.
- **Piper — Amy (English, US, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Ryan (English, US, high)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Lessac (English, US, high)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Alba (English, GB, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Northern English Male (English, GB, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Thorsten (German, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — Siwis (French, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Piper — DaveFX (Spanish, medium)** (`user-loaded-only`): Manual install; per-voice license varies (VOICES.md). PortBay ships nothing.
- **Stable Diffusion 2.1 (base)** (`clear-with-condition`): RAIL condition = downstream use restrictions travel with the weights; distribution is permitted and the license passes through at download.
- **Stable Diffusion 1.5** (`clear-with-condition`): RAIL condition = downstream use restrictions travel with the weights; distribution permitted.
- **SD-Turbo** (`clear-with-condition`): Stability AI Community License: free for commercial use below the annual-revenue threshold (currently US$1M); above it, a Stability enterprise license is required. Condition disclosed at download.
- **Stable Diffusion XL (base)** (`clear-with-condition`): RAIL condition = downstream use restrictions travel with the weights; distribution permitted.
- **Presence pet — Finder (spritesheet + preview)** (`accepted-risk-with-rationale`): Owner-original art depicting the Apple 'Finder' two-tone face likeness. The owner made an EXPLICIT keep decision on 2026-07-17 accepting the Apple-likeness trademark/trade-dress risk (parody/homage; small pixel mascot; no Apple branding, wordmark, or claim of affiliation). Recorded as accepted risk per owner directive. Bundled at static/presence/pets/finder/.
- **Presence pet — Macintosh (spritesheet + preview)** (`accepted-risk-with-rationale`): Owner-original art depicting a classic Macintosh likeness with a Finder smile. The owner made an EXPLICIT keep decision on 2026-07-17 accepting the Apple-likeness/trade-dress risk (nostalgic homage; small pixel mascot; no Apple branding, wordmark, or claim of affiliation). Recorded as accepted risk per owner directive. Bundled at static/presence/pets/macintosh/.
- **Presence pet — Yee (spritesheet + preview)** (`accepted-risk-with-rationale`): Owner-original art at static/presence/pets/yeelight-scene-screen-commander/ — ADJUDICATED 2026-07-17 (COMP-5 gap closure). Art inspected: a generic dark robot with a glowing screen face and neon accents; it depicts NO Yeelight/Xiaomi product, logo, or trade dress — the art is clear. Residual risk is NAMING-ONLY: the internal asset id/directory 'yeelight-scene-screen-commander' embeds the Yeelight (Xiaomi) brand string. Rationale for accepted risk: the user-facing name is 'Yee' only (petCatalog.ts), the brand string never renders in UI or marketing, appears solely in an asset path, and implies no affiliation — LOW residual trademark exposure. RECOMMENDATION (recorded for the owner, not executed — code rename out of COMP-5 scope): rename the asset id/directory to 'yee' at the next asset-touching change to remove the brand string entirely.
- **openWakeWord pre-trained wake classifiers (hey_jarvis, alexa, hey_mycroft, …)** (`excluded`): NonCommercial (training data of unknown/restrictive license). NEVER bundled — PortBay ships only the Apache-2.0 extractors + its own hey_portbay.onnx classifier. Verified absent from src-tauri/resources/wake/.
- **TEN-VAD** (`excluded`): On-device hosting/self-hosting is forbidden by its license. EXCLUDED — Silero VAD (MIT, bundled) is the clean VAD path. Not referenced by any shipping code.
- **Cohere Transcribe 03-2026 CoreML (formerly catalog id `cohere-transcribe`) — REMOVED from catalog 2026-07-17** (`excluded`): RESOLVED 2026-07-17 (COMP-5) by REMOVING the entry from default-model-catalog.json. Research findings: the UPSTREAM CohereLabs/cohere-transcribe-03-2026 is Apache-2.0 (model-card metadata + the official Cohere Labs release post https://huggingface.co/blog/CohereLabs/cohere-transcribe-03-2026-release both state Apache 2.0) but the upstream repo is GATED (contact-info agreement required). The artifact PortBay's sidecar would actually fetch — FluidInference/cohere-transcribe-03-2026-coreml — SELF-CONTRADICTS: its README YAML frontmatter declares `license: apache-2.0` while its README License section states "CC-BY-NC-4.0 (matches original Cohere Transcribe model)" (a claim that mismatches the actual Apache-2.0 upstream). Because the fetched artifact's governing terms are ambiguous and the conservative reading is NonCommercial, the entry is EXCLUDED from the catalog rather than shipped. RE-ADD PATH: only after FluidInference resolves its license contradiction in favor of Apache-2.0, or PortBay stages its own conversion directly from the Apache-2.0 upstream — and NOT via the hosted live catalog without a register row first (merge_stt lets live-catalog entries introduce ids the bundled catalog lacks). Engine plumbing for `engine: "cohere"` remains in the app but is inert with no catalog entry.
- **LiveKit turn-detector weights** (`excluded`): License forbids standalone use and distillation. EXCLUDED — Smart Turn v3 (BSD-2, bundled) + Parakeet acoustic-EOU (CC-BY-4.0) are the clean turn/EOU paths. Not referenced by any shipping code.
- **Porcupine wake engine (Picovoice)** (`excluded`): Proprietary — not bundled and not used. The Apache-2.0 openWakeWord extractor + owner-trained classifier are the wake path instead.
- **xLAM (Salesforce function-calling LLM)** (`excluded`): NonCommercial. EXCLUDED — never bundled or fetched by PortBay; cited only as a known NC landmine in src-tauri/src/voice_wake.rs. If a user manually pulls it via Ollama it is their own user-loaded model, outside PortBay's distribution.
- **Ollama-pulled local LLMs (user-selected)** (`user-loaded-only`): PortBay neither bundles nor redistributes any Ollama model; the user pulls models through Ollama and each model's license governs. Recommended defaults are permissive (e.g. Qwen3 Apache-2.0). PortBay bears no redistribution obligation for user-pulled Ollama weights.

---

Machine-readable form: `THIRD-PARTY-MODELS.json`. Bundled non-model third-party software (sidecar binaries, statically-linked AI libraries) is covered in `NOTICE`; the full dependency SBOM is `portbay-sbom.cdx.json`.
