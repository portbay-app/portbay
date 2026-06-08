#!/bin/bash
# A/B counterpart to probe.sh: same framed request against a local Ollama
# (the app's OllamaProvider parameters — temperature 0.0, greedy: 0.2 left
# run-to-run variance that flipped self-corrections, j11 2026-06-06; greedy
# is also what makes the "dictation rewrite input" breadcrumb exactly
# reproducible here). Server must run:
#   OLLAMA_MODELS=/Volumes/DevSSD/system/ai/models/ollama ollama serve
# Usage: ollama-probe.sh <system-prompt-file> <transcript-file> [model]
set -euo pipefail
SYSTEM=$(cat "$1"); PROMPT="Transcript: $(cat "$2")"; MODEL=${3:-qwen2.5:7b}
# think:false is app-exact (OllamaProvider::rewrite): a reasoning model
# (qwen3.x, deepseek-r1) otherwise spends the whole num_predict budget thinking
# and returns an EMPTY response (probed 2026-06-08, qwen3.5:9b). Ollama ignores
# it on non-reasoning models.
jq -n --arg system "$SYSTEM" --arg prompt "$PROMPT" --arg model "$MODEL" \
  '{model:$model, system:$system, prompt:$prompt, stream:false, think:false, keep_alive:"15m", options:{temperature:0.0, num_predict:800}}' \
  | curl -s http://127.0.0.1:11434/api/generate -d @- | jq -r '.response'
