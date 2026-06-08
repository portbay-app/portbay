<!--
  TlsSourceFields — the SSL-mode picker and its conditional inputs (custom cert
  paths, public-ACME issuer/EAB/DNS, auto-manage + wildcard toggles).

  Renders everything below the "Enable HTTPS" switch; the caller owns the HTTPS
  toggle and binds `draft.https` here so the fields disable when HTTPS is off.
  Used by the Add Certificate panel; the Certificates and Domains pages keep
  their inline copies for now.
-->
<script lang="ts">
  import Icon from "$lib/components/atoms/Icon.svelte";
  import {
    acmeEnvironmentOptions,
    acmeIssuerOptions,
    acmeKeyTypeOptions,
    dnsProviderOptions,
    sslModeOptions,
    type TlsDraft,
  } from "$lib/certs/tlsDraft";

  let {
    draft = $bindable(),
    hostname,
    idPrefix = "tls",
  }: { draft: TlsDraft; hostname: string; idPrefix?: string } = $props();
</script>

{#snippet toggle(on: boolean, flip: () => void, label: string, disabled = false)}
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-label={label}
    {disabled}
    onclick={flip}
    class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full
           transition-colors active:scale-95 focus-visible:outline-none
           focus-visible:ring-2 focus-visible:ring-accent/40 disabled:opacity-45
           disabled:cursor-not-allowed
           {on ? 'bg-accent' : 'bg-surface-2 border border-border'}"
  >
    <span
      class="inline-block h-3.5 w-3.5 rounded-full bg-white shadow-sm
             transition-transform duration-150
             {on ? 'translate-x-[18px]' : 'translate-x-0.5'}"
    ></span>
  </button>
{/snippet}

{#snippet toggleRow(
  label: string,
  help: string,
  on: boolean,
  flip: () => void,
  disabled = false,
)}
  <div class="flex items-start justify-between gap-4 py-2">
    <div class="min-w-0">
      <div class="text-[12.5px] text-fg">{label}</div>
      <div class="mt-0.5 text-[11px] text-fg-subtle leading-relaxed">{help}</div>
    </div>
    {@render toggle(on, flip, label, disabled)}
  </div>
{/snippet}

<div class="space-y-4">
  <div class="space-y-1.5">
    <label for="{idPrefix}-ssl-mode" class="text-[12px] font-medium text-fg-muted">
      SSL mode
    </label>
    <select
      id="{idPrefix}-ssl-mode"
      bind:value={draft.sslMode}
      onchange={() => {
        draft.autoManageCert =
          draft.https && draft.sslMode === "automatic_local";
      }}
      disabled={!draft.https}
      class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
             text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
             disabled:opacity-50"
    >
      {#each sslModeOptions as o (o.value)}
        <option value={o.value}>{o.label}</option>
      {/each}
    </select>
    <p class="text-[11px] text-fg-subtle leading-relaxed">
      {#if !draft.https}
        Turn on HTTPS to create a certificate for this project.
      {:else if draft.sslMode === "automatic_local"}
        PortBay issues a locally trusted mkcert certificate and keeps its SANs
        aligned with this hostname.
      {:else if draft.sslMode === "custom_certificate"}
        Use a company or hand-issued certificate. The cert and key must match and
        cover this hostname.
      {:else if draft.sslMode === "self_signed"}
        Intended only as a fallback. Browsers will show warnings until you switch
        back to a trusted mode.
      {:else}
        Reserved for public domains only; local
        <code class="font-mono">.test</code> names are not eligible for public ACME.
      {/if}
    </p>
  </div>

  {#if draft.sslMode === "custom_certificate"}
    <div class="grid grid-cols-1 gap-3">
      <div class="space-y-1.5">
        <label for="{idPrefix}-custom-cert" class="text-[12px] font-medium text-fg-muted">
          Certificate path
        </label>
        <input
          id="{idPrefix}-custom-cert"
          bind:value={draft.customCertPath}
          disabled={!draft.https}
          placeholder="/absolute/path/cert.pem"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
      </div>
      <div class="space-y-1.5">
        <label for="{idPrefix}-custom-key" class="text-[12px] font-medium text-fg-muted">
          Private key path
        </label>
        <input
          id="{idPrefix}-custom-key"
          bind:value={draft.customKeyPath}
          disabled={!draft.https}
          placeholder="/absolute/path/key.pem"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
      </div>
    </div>
  {/if}

  {#if draft.sslMode === "public_acme"}
    <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
      <div class="space-y-1.5">
        <label for="{idPrefix}-acme-issuer" class="text-[12px] font-medium text-fg-muted">
          Issuer
        </label>
        <select
          id="{idPrefix}-acme-issuer"
          bind:value={draft.acmeIssuer}
          disabled={!draft.https}
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        >
          {#each acmeIssuerOptions as o (o.value)}
            <option value={o.value}>{o.label}</option>
          {/each}
        </select>
      </div>
      <div class="space-y-1.5">
        <label for="{idPrefix}-acme-env" class="text-[12px] font-medium text-fg-muted">
          Environment
        </label>
        <select
          id="{idPrefix}-acme-env"
          bind:value={draft.acmeEnvironment}
          disabled={!draft.https}
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        >
          {#each acmeEnvironmentOptions as o (o.value)}
            <option value={o.value}>{o.label}</option>
          {/each}
        </select>
      </div>
      <div class="space-y-1.5">
        <label for="{idPrefix}-acme-email" class="text-[12px] font-medium text-fg-muted">
          Account email
        </label>
        <input
          id="{idPrefix}-acme-email"
          type="email"
          bind:value={draft.acmeEmail}
          disabled={!draft.https}
          placeholder="admin@example.com"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
      </div>
      <div class="space-y-1.5">
        <label for="{idPrefix}-acme-key" class="text-[12px] font-medium text-fg-muted">
          Algorithm
        </label>
        <select
          id="{idPrefix}-acme-key"
          bind:value={draft.acmeKeyType}
          disabled={!draft.https}
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        >
          {#each acmeKeyTypeOptions as o (o.value)}
            <option value={o.value}>{o.label}</option>
          {/each}
        </select>
      </div>
      <div class="space-y-1.5">
        <label for="{idPrefix}-acme-dns-provider" class="text-[12px] font-medium text-fg-muted">
          DNS API provider
        </label>
        <select
          id="{idPrefix}-acme-dns-provider"
          bind:value={draft.acmeDnsProvider}
          disabled={!draft.https}
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border text-[13px]
                 text-fg focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        >
          {#each dnsProviderOptions as o (o.value)}
            <option value={o.value}>{o.label}</option>
          {/each}
        </select>
      </div>
      <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
        {@render toggleRow(
          "Enable debug",
          "Emit extra ACME diagnostics from Caddy.",
          draft.acmeDebug,
          () => (draft.acmeDebug = !draft.acmeDebug),
          !draft.https,
        )}
        {@render toggleRow(
          "Force request",
          "Force Caddy to attempt issuance again on the next reload.",
          draft.acmeForceRequest,
          () => (draft.acmeForceRequest = !draft.acmeForceRequest),
          !draft.https,
        )}
      </div>
    </div>

    {#if draft.acmeIssuer === "zero_ssl"}
      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        <input
          aria-label="ZeroSSL API key"
          bind:value={draft.acmeZerosslApiKey}
          disabled={!draft.https}
          placeholder="ZeroSSL API key"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
        <p class="text-[11px] text-fg-subtle leading-relaxed">
          ZeroSSL can use its Caddy issuer API key, or EAB key id and HMAC below.
        </p>
      </div>
    {/if}

    {#if draft.acmeIssuer === "zero_ssl" || draft.acmeIssuer === "google_trust_services"}
      <div class="grid grid-cols-1 md:grid-cols-2 gap-3">
        <input
          aria-label="ACME EAB key id"
          bind:value={draft.acmeEabKeyId}
          disabled={!draft.https}
          placeholder="EAB key id"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
        <input
          aria-label="ACME EAB HMAC key"
          type="password"
          bind:value={draft.acmeEabHmacKey}
          disabled={!draft.https}
          placeholder="EAB HMAC key"
          class="w-full h-9 px-3 rounded-lg bg-bg border border-border
                 text-[13px] text-fg font-mono placeholder:text-fg-subtle
                 focus:outline-none focus:ring-2 focus:ring-accent/40
                 disabled:opacity-50"
        />
      </div>
    {/if}

    {#if draft.acmeDnsProvider === "cloudflare"}
      <textarea
        aria-label="Cloudflare DNS API token"
        bind:value={draft.acmeDnsApiToken}
        disabled={!draft.https}
        rows="3"
        placeholder="Cloudflare API token with Zone:DNS:Edit for this domain"
        class="w-full px-3 py-2 rounded-lg bg-bg border border-border
               text-[13px] text-fg font-mono placeholder:text-fg-subtle
               focus:outline-none focus:ring-2 focus:ring-accent/40
               disabled:opacity-50"
      ></textarea>
      <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
        Use a scoped token with Zone:Zone:Read and Zone:DNS:Edit for this zone.
        Wildcard public certificates use Cloudflare DNS-01.
      </p>
    {/if}
  {/if}

  <div class="rounded-xl border border-border divide-y divide-border/60 px-4">
    {@render toggleRow(
      "Auto-manage certificate",
      "PortBay issues and renews this hostname's local certificate.",
      draft.autoManageCert,
      () => (draft.autoManageCert = !draft.autoManageCert),
      !draft.https || draft.sslMode !== "automatic_local",
    )}
    {@render toggleRow(
      "Include wildcard subdomains",
      "Also route and certify *." + hostname + ".",
      draft.includeWildcardSubdomains,
      () => (draft.includeWildcardSubdomains = !draft.includeWildcardSubdomains),
      !draft.https,
    )}
  </div>

  {#if draft.https && draft.sslMode === "automatic_local" && !draft.autoManageCert}
    <p
      class="flex items-start gap-2 text-[11.5px] text-status-unhealthy
             bg-status-unhealthy/10 rounded-lg px-3 py-2 leading-relaxed"
    >
      <Icon name="circle-alert" size={13} class="mt-px shrink-0" />
      HTTPS is on but certificate auto-management is off. Re-enable it, or provide
      another TLS source.
    </p>
  {/if}
  {#if draft.includeWildcardSubdomains}
    <p class="text-[11px] text-fg-subtle leading-relaxed -mt-2">
      Subdomains resolve only under the dnsmasq wildcard resolver.
    </p>
  {/if}
</div>
