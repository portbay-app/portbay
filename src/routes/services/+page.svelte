<script lang="ts">
  import { DashboardCard } from "$lib/components/atoms";
</script>

<div class="p-6 space-y-4">
  <DashboardCard title="Services" flush>
    <p class="text-sm text-fg-muted">
      Shared service health (Caddy, dnsmasq, mkcert, process-compose) will
      surface here in full detail in card #5. The top-of-Projects sidecar
      row already shows the compact view.
    </p>
  </DashboardCard>
</div>
