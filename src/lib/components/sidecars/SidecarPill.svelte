<!--
  SidecarPill — compact, always-visible variant for the sidebar footer.

  Renders a single overall-health dot + word. The dot picks the worst
  state across all sidecars so a single glance answers "is anything
  broken?" The full per-sidecar breakdown lives in the dashboard row.
-->
<script lang="ts">
  import StatusDot from "$lib/components/atoms/StatusDot.svelte";
  import { sidecars } from "$lib/stores/sidecars.svelte";
  import type { PortbayStatus } from "$lib/types/status";
  import type { SidecarState } from "$lib/types/sidecars";
  import { SIDECAR_ORDER } from "$lib/types/sidecars";

  // Priority of worst → best. The first state in this list that any
  // sidecar reports becomes the pill's status.
  const SEVERITY: SidecarState[] = [
    "unreachable",
    "not_installed",
    "stopped",
    "running",
  ];

  const summary = $derived.by(() => {
    const states = SIDECAR_ORDER.map((k) => sidecars.value[k].status);
    for (const candidate of SEVERITY) {
      if (states.includes(candidate)) return candidate;
    }
    return "running";
  });

  const pillStatus = $derived.by<PortbayStatus>(() => {
    switch (summary) {
      case "running":
        return "running";
      case "stopped":
        return "stopped";
      case "not_installed":
        return "port_conflict";
      case "unreachable":
        return "crashed";
    }
  });

  const label = $derived.by(() => {
    switch (summary) {
      case "running":
        return "ready";
      case "stopped":
        return "idle";
      case "not_installed":
        // Surfaces when a sidecar binary isn't bundled / on PATH.
        // Friendly wording — "missing tools" reads as a hard error
        // even when it's just a one-click setup away.
        return "setup needed";
      case "unreachable":
        return "daemon down";
    }
  });
</script>

<span class="inline-flex items-center gap-1.5 text-[11px] text-fg-muted">
  <StatusDot status={pillStatus} size="sm" />
  <span>{label}</span>
</span>
