<!--
  AccountPanel — the "Account" tab.

  Composes the self-contained account surfaces (each renders its own titled
  card): account/plan + profile, Sync (Pro), and Early Access (Pro). No
  SettingsPanel header here — that would duplicate their built-in titles.
-->
<script lang="ts">
  import {
    AccountSection,
    SyncSection,
    EarlyAccessSection,
  } from "$lib/components/account";
</script>

<div class="max-w-3xl space-y-6">
  <AccountSection />
  <SyncSection />
  <EarlyAccessSection />
</div>
