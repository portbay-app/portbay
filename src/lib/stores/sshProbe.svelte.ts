import { browser } from "$app/environment";

import { invokeQuiet } from "$lib/ipc";
import type { ProbeResult } from "$lib/types/sshConnections";

/**
 * Live reachability/health probes for SSH hosts, keyed by connection id.
 *
 * A probe is one unauthenticated handshake (see `ssh/probe.rs`) that reports
 * reachability, latency, host-key fingerprint, and trust. Results are cached
 * here so selecting a host in the panel reuses the dashboard's probe rather
 * than re-dialling. Probing is explicit — on page mount and the table's refresh
 * button — never a background poll.
 */
function createSshProbeStore() {
  let results = $state<Record<string, ProbeResult>>({});
  let inflight = $state<Record<string, boolean>>({});

  /** The cached probe for a host, or null if it hasn't been probed yet. */
  function get(id: string): ProbeResult | null {
    return results[id] ?? null;
  }

  function isProbing(id: string): boolean {
    return inflight[id] === true;
  }

  /** Probe one host, caching the result. Concurrent calls for the same id coalesce. */
  async function probe(id: string): Promise<ProbeResult | null> {
    if (!browser || inflight[id]) return results[id] ?? null;
    inflight = { ...inflight, [id]: true };
    try {
      const result = await invokeQuiet<ProbeResult>("ssh_connection_probe", { id });
      results = { ...results, [id]: result };
      return result;
    } catch {
      // A probe shouldn't surface a toast — an unreachable host is a normal
      // state, not an error. Record it as indeterminate.
      const fallback: ProbeResult = {
        reachable: false,
        latencyMs: null,
        health: "unknown",
        fingerprint: null,
        trust: "unknown",
      };
      results = { ...results, [id]: fallback };
      return fallback;
    } finally {
      inflight = { ...inflight, [id]: false };
    }
  }

  /** Probe many hosts concurrently (page mount / refresh button). */
  async function probeAll(ids: string[]): Promise<void> {
    if (!browser) return;
    await Promise.allSettled(ids.map((id) => probe(id)));
  }

  return {
    get,
    isProbing,
    probe,
    probeAll,
  };
}

export const sshProbe = createSshProbeStore();
