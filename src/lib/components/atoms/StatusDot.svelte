<!--
  StatusDot — colored circle prefixing a project / service name.

  Lifted from Lerd (`internal/ui/web/src/components/StatusDot.svelte`, MIT)
  and adapted to PortBay's status taxonomy. See NOTICE for attribution.

  Strict status type: the prop is the same `PortbayStatus` used everywhere
  else (Rust ↔ TS), not a Lerd-style color string. Status word and color
  must always agree (docs/UX_DESIGN.md §5.3).
-->
<script lang="ts" module>
  export type DotSize = "sm" | "md" | "lg";
</script>

<script lang="ts">
  import type { DisplayStatus } from "$lib/types/status";
  import { statusDotClass, isTransitional } from "$lib/types/status";

  interface Props {
    status: DisplayStatus;
    size?: DotSize;
    /** Pulse animation for transitional states (starting / stopping). */
    pulse?: boolean;
  }
  let { status, size = "md", pulse = false }: Props = $props();

  const sizeClass: Record<DotSize, string> = {
    sm: "w-1.5 h-1.5",
    md: "w-2 h-2",
    lg: "w-2.5 h-2.5",
  };

  const cls = $derived(
    `${sizeClass[size]} ${statusDotClass(status)} rounded-full shrink-0${
      pulse || isTransitional(status) ? " animate-pulse" : ""
    }`,
  );
</script>

<span class={cls} aria-hidden="true"></span>
