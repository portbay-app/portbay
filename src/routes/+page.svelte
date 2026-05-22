<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  type Proc = {
    name: string;
    status: string;
    is_running: boolean;
    pid?: number;
    is_ready?: string;
    restarts?: number;
  };

  let alive = $state<boolean | null>(null);
  let procs = $state<Proc[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function refresh() {
    loading = true;
    error = null;
    try {
      alive = await invoke<boolean>("pc_alive");
      if (alive) {
        const res = await invoke<{ data: Proc[] }>("pc_processes");
        procs = res.data ?? [];
      } else {
        procs = [];
      }
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
</script>

<main class="container">
  <header>
    <h1>PortBay</h1>
    <p class="subtitle">Tauri 2 + Rust core + Process Compose sidecar spike</p>
  </header>

  <section class="actions">
    <button onclick={refresh} disabled={loading}>
      {loading ? "Loading…" : "Refresh"}
    </button>
    <span class="status" class:ok={alive === true} class:bad={alive === false}>
      {#if alive === null}
        not checked
      {:else if alive}
        daemon alive
      {:else}
        daemon down
      {/if}
    </span>
  </section>

  {#if error}
    <pre class="error">{error}</pre>
  {/if}

  {#if procs.length === 0 && !error}
    <p class="empty">No processes yet — click Refresh.</p>
  {:else}
    <table>
      <thead>
        <tr>
          <th>Name</th>
          <th>Status</th>
          <th>PID</th>
          <th>Ready</th>
          <th>Restarts</th>
        </tr>
      </thead>
      <tbody>
        {#each procs as p}
          <tr>
            <td>{p.name}</td>
            <td class:running={p.is_running}>{p.status}</td>
            <td>{p.pid ?? "—"}</td>
            <td>{p.is_ready ?? "—"}</td>
            <td>{p.restarts ?? 0}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, "SF Pro Text", Inter, sans-serif;
    background: #fafafa;
    color: #1a1a1a;
  }
  @media (prefers-color-scheme: dark) {
    :global(body) {
      background: #1a1a1a;
      color: #fafafa;
    }
  }

  .container {
    max-width: 960px;
    margin: 0 auto;
    padding: 2rem 1.5rem;
  }
  header h1 {
    margin: 0 0 0.25rem 0;
    font-size: 1.6rem;
    font-weight: 600;
  }
  .subtitle {
    margin: 0 0 1.5rem 0;
    color: #888;
    font-size: 0.85rem;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1.5rem;
  }
  button {
    padding: 0.4rem 0.9rem;
    border-radius: 6px;
    border: 1px solid #d0d0d0;
    background: white;
    cursor: pointer;
    font-size: 0.85rem;
  }
  button:disabled { opacity: 0.5; cursor: not-allowed; }
  @media (prefers-color-scheme: dark) {
    button { background: #2a2a2a; border-color: #3a3a3a; color: inherit; }
  }
  .status {
    font-size: 0.8rem;
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    background: #eee;
  }
  .status.ok { background: #d6f5dd; color: #1a6b2f; }
  .status.bad { background: #f5d6d6; color: #8a1a1a; }
  .error {
    background: #f5d6d6;
    color: #8a1a1a;
    padding: 0.6rem;
    border-radius: 6px;
    font-size: 0.8rem;
    overflow-x: auto;
  }
  .empty { color: #888; font-size: 0.9rem; }
  table { width: 100%; border-collapse: collapse; font-size: 0.85rem; }
  th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #eaeaea; }
  @media (prefers-color-scheme: dark) {
    th, td { border-bottom-color: #2a2a2a; }
  }
  th { font-weight: 600; color: #666; }
  td.running { color: #1a6b2f; font-weight: 500; }
</style>
