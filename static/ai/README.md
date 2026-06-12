# AI model-family brand marks

Logos rendered by `src/lib/components/atoms/ModelMark.svelte` for the AI
page's model-catalog family list. All assets are bundled — the desktop app
never hotlinks logo CDNs.

## Sources & licenses

**Simple Icons** (<https://simpleicons.org>, CC0 1.0 — public domain).
Exported from the `simple-icons` npm package; each glyph is tinted with the
brand colour Simple Icons publishes for it (`fill="#…"` stamped on the root
`<svg>`):

- `qwen.svg` (QWen) — Qwen 2.5 / Qwen 3 families
- `meta.svg` (Meta) — Llama family
- `deepseek.svg` (DeepSeek)
- `moonshot.svg` (Moonshot AI) — Kimi family
- `google.svg` (Google) — Gemma family
- `mistral.svg` (Mistral AI)
- `lg.svg` (LG) — EXAONE family
- `ollama.svg` (Ollama) — the "More models" catch-all
- `nvidia.svg` (NVIDIA) — Parakeet + Nemotron speech-to-text engines

## Ids with no asset (brand-tinted monogram chips)

Simple Icons no longer carries these marks (Microsoft pulled theirs; several
AI labs requested removal), so they render as monogram chips in ModelMark:

- Phi (Microsoft) — chip `Phi`
- Whisper (OpenAI) — chip `Wh`
- Cohere — chip `Co`
- Kokoro (TTS) — chip `Ko`
- FLUX (Black Forest Labs) — chip `FL`
- Stable Diffusion / SDXL (Stability AI) — chip `SD`
- Embeddings / Vision — functional groupings, not brands; render as neutral
  Icon glyphs (search / eye).

All trademarks and logos remain the property of their respective owners; they
identify the model families a catalog entry belongs to, not affiliation or
endorsement.
