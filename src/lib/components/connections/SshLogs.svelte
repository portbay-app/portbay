<!--
  SshLogs — follow a remote log over the pty stream. Pick a file (`tail -n 200
  -F <path>`) or the systemd journal (`journalctl -n 200 -f [-u unit]`), hit
  Follow, and the output streams live into a read-only terminal view. Stop
  unmounts the session, which closes the channel and ends the follow on the host.

  Reuses SshTerminalSession (command + read-only) so the streaming path and
  credential flow are exactly the terminal's — no duplicate plumbing.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import SshTerminalSession from "$lib/components/connections/SshTerminalSession.svelte";

  let { connectionId, label }: { connectionId: string; label: string } = $props();

  type Source = "file" | "journal";
  let source = $state<Source>("file");
  let path = $state("/var/log/syslog");
  let unit = $state("");
  let following = $state(false);
  let runKey = $state(0);

  const FILE_PRESETS = [
    "/var/log/syslog",
    "/var/log/messages",
    "/var/log/nginx/error.log",
    "/var/log/nginx/access.log",
  ];

  /** POSIX single-quote a path/unit so spaces and metacharacters are literal. */
  function shellQuote(value: string): string {
    if (/^[A-Za-z0-9_./:@=+,-]+$/.test(value)) return value;
    return `'${value.replaceAll("'", "'\\''")}'`;
  }

  const command = $derived.by(() => {
    if (source === "journal") {
      const u = unit.trim();
      return `journalctl -n 200 -f${u ? ` -u ${shellQuote(u)}` : ""}`;
    }
    const p = path.trim();
    return p ? `tail -n 200 -F ${shellQuote(p)}` : "";
  });

  function follow() {
    if (!command) return;
    runKey += 1;
    following = true;
  }

  function stop() {
    following = false;
  }
</script>

<div class="flex h-full min-h-0 flex-col">
  <header class="border-b border-border/60 px-6 py-3">
    <div class="flex items-center gap-2">
      <Icon name="file-text" size={15} class="text-fg-muted" />
      <h2 class="text-[13px] font-semibold text-fg">Logs</h2>
      <p class="text-[11px] text-fg-subtle">Live follow over SSH — read-only.</p>
      {#if following}
        <button
          type="button"
          onclick={stop}
          class="ml-auto inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium border border-status-crashed/40 text-status-crashed hover:bg-status-crashed/10"
        >
          <Icon name="circle-stop" size={12} /> Stop
        </button>
      {:else}
        <button
          type="button"
          onclick={follow}
          disabled={!command}
          class="ml-auto inline-flex items-center gap-1.5 h-8 px-3 rounded-md text-[12px] font-medium bg-accent text-on-accent hover:brightness-110 disabled:opacity-50"
        >
          <Icon name="play" size={12} /> Follow
        </button>
      {/if}
    </div>

    <div class="mt-3 flex flex-wrap items-center gap-2">
      <div class="inline-flex rounded-md border border-border p-0.5">
        <button
          type="button"
          onclick={() => (source = "file")}
          class="h-7 px-2.5 rounded text-[11.5px] font-medium {source === 'file' ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:text-fg'}"
        >
          File
        </button>
        <button
          type="button"
          onclick={() => (source = "journal")}
          class="h-7 px-2.5 rounded text-[11.5px] font-medium {source === 'journal' ? 'bg-surface-2 text-fg' : 'text-fg-muted hover:text-fg'}"
        >
          Journal
        </button>
      </div>

      {#if source === "file"}
        <input
          bind:value={path}
          placeholder="/var/log/syslog"
          class="h-8 min-w-0 flex-1 rounded-md border border-border bg-surface px-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
        />
      {:else}
        <input
          bind:value={unit}
          placeholder="unit (optional, e.g. nginx.service)"
          class="h-8 min-w-0 flex-1 rounded-md border border-border bg-surface px-2 font-mono text-[12px] text-fg outline-none focus:border-accent"
        />
      {/if}
    </div>

    {#if source === "file"}
      <div class="mt-2 flex flex-wrap items-center gap-1.5">
        {#each FILE_PRESETS as preset (preset)}
          <button
            type="button"
            onclick={() => (path = preset)}
            class="rounded bg-surface-2 px-2 py-0.5 font-mono text-[10.5px] text-fg-muted hover:text-fg"
          >
            {preset}
          </button>
        {/each}
      </div>
    {/if}
  </header>

  <div class="min-h-0 flex-1">
    {#if following}
      {#key runKey}
        <SshTerminalSession {connectionId} {label} {command} disableInput active />
      {/key}
    {:else}
      <div class="flex h-full items-center justify-center px-6 text-center">
        <p class="max-w-sm text-[12px] text-fg-subtle leading-relaxed">
          Choose a file or the journal and press <span class="text-fg">Follow</span> to stream it
          live. Stopping ends the follow on the host.
        </p>
      </div>
    {/if}
  </div>
</div>
