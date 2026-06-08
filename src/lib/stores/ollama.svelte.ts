/**
 * Ollama running-state store — a lightweight, app-wide signal for whether
 * an Ollama server is alive (managed, recorded-orphan, or external). The AI
 * page polls the full `ollama_overview` for its rich UI; this store exists so
 * surfaces outside that page — primarily the TopBar's "Stop All" button —
 * know Ollama is running without the AI page being mounted. It polls the cheap
 * `ollama_running` probe (no disk scan / model listing) every 5 s, mirroring
 * the global `tunnels` store's lifecycle.
 */
import { browser } from "$app/environment";

import { invokeQuiet } from "$lib/ipc";
import type { OllamaLoadedModel } from "$lib/types/ai";

const POLL_INTERVAL_MS = 5_000;

function createOllamaStore() {
  let running = $state(false);
  let loaded = $state<OllamaLoadedModel[]>([]);
  let timer: ReturnType<typeof setInterval> | null = null;

  async function refresh(): Promise<void> {
    if (!browser) return;
    try {
      running = await invokeQuiet<boolean>("ollama_running");
    } catch {
      // A quiet probe — no toast. An IPC race on restart shouldn't nag every
      // 5 s; leave the last-known value in place so the Stop-All button doesn't
      // flicker on a transient probe failure.
      return;
    }
    // Loaded-model list feeds the dashboard's Local AI card. Probe it only
    // while the server answers — `/api/ps` on a dead endpoint just errors, and
    // a stopped server has nothing loaded — so an idle app stays as cheap as
    // the bare running check.
    if (running) {
      try {
        loaded = await invokeQuiet<OllamaLoadedModel[]>("ollama_loaded_models");
      } catch {
        // Transient probe failure — keep the last-known list rather than
        // flickering the card to "no model" on a single missed tick.
      }
    } else if (loaded.length > 0) {
      loaded = [];
    }
  }

  function start() {
    if (!browser || timer !== null) return;
    void refresh();
    timer = setInterval(() => void refresh(), POLL_INTERVAL_MS);
  }

  function stop() {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  }

  return {
    /** True when an Ollama server is alive — what Stop All would shut down. */
    get running() {
      return running;
    },
    /** Models currently held in memory (`/api/ps`), refreshed while running. */
    get loaded() {
      return loaded;
    },
    refresh,
    start,
    stop,
  };
}

export const ollamaService = createOllamaStore();
