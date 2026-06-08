# probe-afm — offline probe kit for the Smart Dictation prompt

The rewrite prompt in `src-tauri/src/dictation.rs` (`BASE_RULES`,
`SMART_EXAMPLES`, the per-context tails, `build_user`'s `Transcript:`
framing) is **load-bearing and tuned to the on-device 3B's ceiling**. Probed
evidence (2026-06-06): rules alone are inert on this model — examples are the
only lever; concrete example content leaks verbatim into outputs; even the
example HEADER wording perturbs results (changing "A, B, C" to "A, B, C, D"
changed an unrelated transcript's output). v17 and v18 both regressed and
were discarded — v16 is the shipped ceiling.

**The rule: re-probe after ANY wording change, however small.** This kit is
that manual gate. It needs Apple Intelligence hardware (macOS 26+, eligible
device, model downloaded), which is why it is deliberately NOT wired into CI.

## Usage

```sh
# Build the sidecar once (also after editing afm/main.swift):
scripts/build-afm.sh

# Run one transcript against one prompt version (framed like production):
scripts/probe-afm/probe.sh prompts/system-v16.txt transcripts/t1-cancel-that.txt

# Unframed, for experiments only (production always frames — see build_user):
scripts/probe-afm/probe.sh --raw prompts/system-v16.txt transcripts/t3-divide.txt
```

A change is acceptable only if every transcript below still matches its
expected behavior. When the Rust prompt changes, snapshot the new full system
prompt into `prompts/` as the next version (the shipped string and the probed
file must stay byte-identical — verify, don't assume).

## Prompt versions (`prompts/`)

`system-v16.txt` is the SHIPPED prompt head (BASE_RULES + examples; the
`-todo`/`-agent`/`-commit` variants append the context tails, `-vocab`
variants append a vocabulary block). Earlier versions and the rejected
v17/v18 are kept as evidence of what was tried and why it failed:

- v17 — correction-scope + anti-duplication rules: zero effect (rules are
  inert; examples are the only lever).
- v18 — steps-with-inline-corrections example: taught the model to drop the
  WRONG half of corrections ("Create the button into red").
- `system-pass1-*` — the chained correct-then-format experiment, PROBED AND
  REJECTED (2× latency, zero gain; pass-2 re-promoted cancelled leftovers).

### Per-model qwen variant (2026-06-06, round 2)

`system-v16-qwen-agent.txt` / `system-v16-qwen-todo.txt` are the SHIPPED
qwen (Ollama) tails — same v16 head, re-framed AgentPrompt/TodoTask tails
(`PromptFlavor` in dictation.rs keys them by provider kind). Byte-identity
with the Rust strings is pinned by the
`qwen_prompts_match_their_probed_snapshots` unit test — editing either side
without the other fails `cargo test`. Probe with `ollama-probe.sh` (now
temperature 0.0, app-exact: greedy killed j11's correction flakiness and
makes the `dictation rewrite input` breadcrumb exactly reproducible).

Evidence trail (rejected, kept): `system-v16q1*` (extra rules — inert on the
7B too), `q2*` (extra examples — j04 grew a code fence), `q3*`/`q5*`/`q6*`
(tail intermediates), `q7-todo` (concrete "John said" example — folded
t20's lead-in into the list; concrete example content stays banned on the
7B). Root cause of qwen's clause drops was the v16 tail FRAMING
("instruction to an AI coding agent", "one clear, actionable task" →
extract-the-action); rules/examples/emphasis were all inert on those cells.
`system-v16-commit.txt` is the flavor-shared GitCommit tail (assembled for
t14 probing).

### Clean-input layout variant — v19 (2026-06-08)

The rewrite predates the local STT engine and was tuned entirely on macOS
live dictation (raw ASR). The `InputSource` axis in dictation.rs splits that:

- **`Raw`** (macOS dictation) — builds the v16 prompt UNCHANGED. Every probe
  result and pin above still describes this path verbatim. **Nothing about
  Raw moved**, so no re-probe is owed for it.
- **`Clean`** (on-device Whisper/Parakeet via `portbay-stt`) — the transcript
  arrives already punctuated and largely filler-free, so the rewrite's job
  shifts from cleanup to LAYOUT. `build_prompt` appends `CLEAN_LAYOUT_RULES`
  (paragraph grouping on topic shifts) AFTER the context tail, for the prose
  contexts only. GitCommit and TerminalCommand get NOTHING (fixed-shape
  output — paragraph layout would be wrong); the test
  `clean_source_appends_layout_addendum_to_prose_contexts_only` enforces both.

Scope is deliberately PARAGRAPHS only — list/structure behavior stays owned
by the per-context tails (their probed list policy is unchanged). The
addendum also corrects BASE_RULES' "raw speech-to-text" opener, which is
false for this input.

Shipped clean variants, ALL byte-pinned (editing a file or the Rust without
the other fails `cargo test`):

- qwen (Ollama) AgentPrompt tail — `clean_qwen_prompts_match_their_probed_snapshots`:
  `system-v19-clean-qwen-agent.txt`.
- AFM / flavor-shared prose tails — `clean_afm_prose_prompts_match_their_probed_snapshots`:
  `system-v19-clean-general.txt` (GeneralNote), `system-v19-clean-agent.txt`,
  `system-v19-clean-bug.txt` (BugReport), `system-v19-clean-deploy.txt`
  (DeployNote).

Each = the matching v16 tail + a blank line + the addendum. AFM uses the
shared tails for every context (the flavor split only re-frames qwen's
agent/todo), so the AFM files double as the flavor-shared reference.

#### Probe results (2026-06-08) — AFM 3B + Ollama qwen2.5:7b / qwen3.5:9b / phi4:14b

The matrix WAS run. Verdict: the addendum is a net win and BROADER than
expected — its "only arrange, never add/drop/reword a fact" framing doubles as
a no-invention guardrail on the thin prose tails.

- **AFM GeneralNote / BugReport / DeployNote — WIN.** Raw v16 *invented* on all
  three (general answered the note with a fabricated solution list; bug grew a
  5-step repro; deploy turned "docker compose up dash d" into
  `--build-arg=DOCKER_COMPOSE_VERSION=1.23.4`). Clean fixed every one —
  faithful, "dash d" → `-d`, tech names intact (t8).
- **AFM TodoTask — REGRESSION → EXCLUDED.** Clean dropped the speaker's
  attribution ("John said…") that the bare todo tail keeps. TodoTask now gets
  NO addendum on any flavor (`clean_layout_addendum`). Todos are one short
  item — paragraph layout isn't worth losing a fact on the 3B.
- **AFM AgentPrompt — neutral** (clean == raw on t12). Kept; plausible benefit
  on long multi-paragraph prompts, no observed harm.
- **Ollama qwen2.5:7b & phi4:14b — clean works great**, real paragraph layout,
  "John said" survives (the todo drop was AFM-only).
- **Ollama qwen3.5:9b — was EMPTY.** A reasoning model: it spent the entire
  `num_predict` budget in its `thinking` channel, 0 response tokens. **Fixed by
  sending `think:false`** on the rewrite's `/api/generate` call (verified:
  qwen3.5 now returns full faithful text; qwen2.5/phi4 unchanged — Ollama
  ignores the flag on non-reasoning models). This is the dictation path ONLY;
  the coding agent (`context::automation::native`, `/api/chat`) keeps thinking
  ON. `sanitize_output` also strips a leftover leading `<think>…</think>`
  defensively for any model/provider that inlines it.

Re-run after any wording change:

```sh
probe.sh system-v19-clean-general.txt transcripts/t4-rambling.txt    # paragraphs + faithful
probe.sh system-v19-clean-bug.txt     transcripts/j09-ml-overfit.txt # no invented repro
probe.sh system-v19-clean-deploy.txt  transcripts/j04-dev-deploy.txt # "dash d" -> -d, no invention
ollama-probe.sh system-v19-clean-general.txt transcripts/t4-rambling.txt qwen3.5:9b  # add think:false in the script to mirror the app
```

Acceptance bar: a clean transcript reads as well-arranged paragraphs, no facts
added/dropped, and every Raw-path transcript still matches (Raw is byte-identical).

## Transcripts (`transcripts/`) — expected behaviors on v16

| File | Exercises | Expected |
|---|---|---|
| t1-cancel-that | spoken self-correction ("no sorry cancel that") | cancelled content dropped, only final version kept |
| t2-cancel-step | "cancel step three" | the item is removed/replaced; list renumbered in final order |
| t3-divide | instruction-shaped speech | echoed back CLEANED ("Divide this into three steps so it's easier…"), never answered or refused |
| t4-rambling | run-on rambling | compressed into clear prose/list, every fact kept |
| t5-clean-short | already-clean short speech | returned unchanged apart from punctuation/caps |
| t6-prose-no-list | prose that merely mentions items | stays prose — no gratuitous list |
| t7-cancel-all | "scratch all of that" mid-stream | only post-correction content survives |
| t8-technical | paths/flags spoken aloud | technical names verbatim; KNOWN LIMIT: "src dash tauri" may mangle without a vocab anchor |
| t9/t10-instruction* | more instruction shapes (held-out) | echoed cleaned, not answered |
| t11-todo | TodoTask tail | one actionable task + supporting sentences; specifics ("John said…") survive |
| t12/t13-agent | AgentPrompt tail | precise prose-first instruction; lists only for clear enumeration |
| t14-commit | GitCommit tail | imperative ≤72-char summary; no invented type prefix |
| t15-vocab | vocabulary correction | "russ sftp"→`russh-sftp`, "port bay landing"→`portbay-landing` |
| t16-vocab-noinject | unrelated speech + vocab block | ZERO term injection |
| t17-* | chained-pass experiment inputs | (evidence only — chaining rejected) |
| t18-user-real | garbled real transcript | conservative faithful cleanup, NO invented specifics (rule 7) |
| t19-preamble-steps | lead-in sentence + steps | preamble survives ABOVE the numbered list |
| t20-user-real2 | nested mid-item corrections | near-perfect on TodoTask tail; KNOWN LIMIT: meta-renumbering ("make that step four instead") renumbers sequentially |
| t21-user-real3 | preamble + steps + corrections + an UNRELATED vocab block | near-perfect WITHOUT a vocab block; with the full block it degrades ("from red to yellow", folded lead-in) — the evidence behind `anchored_vocabulary`'s spoken-anchor pre-filter. With filtering the block is absent here and the output matches no-vocab |

## Jargon A/B suite (j01–j18)

Cross-industry dictation realism: vibe coders, developers, ML researchers,
small business, CRM/ERP — fillers, corrections, and ASR brand-manglings
(`*-mangled` cases run twice: bare, and with `prompts/jargon-custom-terms.txt`
appended to the tail file to simulate the custom-terms Settings card).
A/B against a local Ollama with `ollama-probe.sh` (same framed request,
app-exact parameters; server must be running — see the
ai-models-location-devssd memory). Full 2026-06-06 results + verdict matrix
(round 1 baseline + round 2 close-out with the qwen variant):
`claudedocs/dictation-jargon-ab-2026-06-06.md`. Qwen probes of
AgentPrompt/TodoTask contexts must use the `system-v16-qwen-*` tails — the
app no longer sends v16's on the Ollama path.

## Voice-command fixtures (`transcripts/cmd-*`, 2026-06-08)

Gap 3 inline voice commands ("new line", "new paragraph", "bullet"/"new
point", "steps"/"next step", "scratch that") are a DETERMINISTIC pre-pass
(`src-tauri/src/dictation_commands.rs`), not model-facing — so these fixtures
are asserted by `cargo test --lib dictation_commands`, not probed against a
model. The pairs:

- `cmd-*-command.txt` — the command spoken as its own clause (fires).
- `cmd-*-content.txt` — the same words inside a real sentence (must NOT fire).

The contract is whole-clause anchoring + byte-identical no-op when nothing
matches, validated against the full `j01`–`j18` + `t1`–`t21` corpus
(`corpus_never_triggers_a_command`): zero spurious extraction, including
`j13`'s run-on "scratch that" self-correction.

## Wire shape

The sidecar takes one JSON request on stdin (one-shot mode):
`{"system": String, "prompt": String, "maxTokens": Int}` → rewritten text on
stdout. The app normally talks to a warm `--serve` process instead (same
request shape, line-delimited; see `afm/main.swift`) — the prompt path is
identical, so one-shot probing remains representative.
