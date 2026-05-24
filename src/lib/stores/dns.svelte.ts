/**
 * DNS management store.
 *
 * Holds the resolver status, the editable dnsmasq settings, and the two
 * read-only lists (DNS records + managed /etc/hosts entries). Mirrors the
 * databases store: `$state` for data, getters for readonly access, methods
 * that wrap `safeInvoke` and refresh in place, and per-action busy markers.
 */
import { browser } from "$app/environment";

import { safeInvoke } from "$lib/ipc";
import { errorBus } from "$lib/stores/errors.svelte";
import { projects } from "$lib/stores/projects.svelte";
import {
  DEFAULT_DNS_SETTINGS,
  type DnsmasqSettings,
  type DnsPreflight,
  type DnsRecord,
  type DomainMigration,
  type ManagedHostsEntry,
  type ResolverStatus,
} from "$lib/types/dns";

function createDnsStore() {
  let status = $state<ResolverStatus | null>(null);
  let settings = $state<DnsmasqSettings>({ ...DEFAULT_DNS_SETTINGS });
  let records = $state<DnsRecord[]>([]);
  let hosts = $state<ManagedHostsEntry[]>([]);
  let preflight = $state<DnsPreflight | null>(null);
  let loading = $state<boolean>(false);

  /** Per-action busy markers keyed by action name. */
  let busy = $state<Record<string, boolean>>({});

  /** Session guard so a Play-triggered setup only auto-prompts once. */
  let autoSetupTried = false;

  function isBusy(action: string): boolean {
    return busy[action] === true;
  }

  function setBusy(action: string, v: boolean) {
    busy = { ...busy, [action]: v };
  }

  async function refresh(): Promise<void> {
    if (!browser) return;
    loading = true;
    try {
      const [s, set, recs, h, pf] = await Promise.all([
        safeInvoke<ResolverStatus>("dnsmasq_resolver_status"),
        safeInvoke<DnsmasqSettings>("get_dnsmasq_settings"),
        safeInvoke<DnsRecord[]>("list_dns_records"),
        safeInvoke<ManagedHostsEntry[]>("list_managed_hosts"),
        safeInvoke<DnsPreflight>("dns_preflight"),
      ]);
      status = s;
      settings = set;
      records = recs;
      hosts = h;
      preflight = pf;
    } catch {
      // safeInvoke pushed the toast.
    } finally {
      loading = false;
    }
  }

  async function saveSettings(next: DnsmasqSettings): Promise<void> {
    if (isBusy("settings")) return;
    setBusy("settings", true);
    try {
      const saved = await safeInvoke<DnsmasqSettings>("set_dnsmasq_settings", {
        settings: next,
      });
      settings = saved;
      errorBus.push({
        code: "DNS_SETTINGS_SAVED",
        whatHappened: "dnsmasq settings saved.",
        whyItMatters:
          "The resolver was restarted with the new cache and TTL configuration.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy("settings", false);
    }
  }

  async function installResolver(): Promise<void> {
    if (isBusy("resolver")) return;
    setBusy("resolver", true);
    try {
      await safeInvoke("dnsmasq_install_resolver");
      errorBus.push({
        code: "DNS_INSTALLED",
        whatHappened: `DNS routing for .${status?.suffix ?? "portbay.test"} installed.`,
        whyItMatters:
          "Subdomains of this suffix now resolve to 127.0.0.1 via dnsmasq — /etc/hosts edits are no longer needed for them.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy("resolver", false);
    }
  }

  async function uninstallResolver(): Promise<void> {
    if (isBusy("resolver")) return;
    setBusy("resolver", true);
    try {
      await safeInvoke("dnsmasq_uninstall_resolver");
      errorBus.push({
        code: "DNS_UNINSTALLED",
        whatHappened: `DNS routing for .${status?.suffix ?? "portbay.test"} removed.`,
        whyItMatters:
          "Hostnames now resolve via /etc/hosts entries managed by PortBay.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy("resolver", false);
    }
  }

  /**
   * One-click first-run setup. Installs PortBay's privileged helper (one
   * macOS password prompt), which then writes /etc/resolver and restarts
   * dnsmasq — after this, *.suffix resolves with no further prompts.
   */
  async function setupLocalDns(): Promise<void> {
    if (isBusy("setup")) return;
    setBusy("setup", true);
    try {
      await safeInvoke("setup_local_dns");
      errorBus.push({
        code: "DNS_SETUP_DONE",
        whatHappened: "Local DNS is set up.",
        whyItMatters:
          "PortBay's privileged helper is installed and your project hostnames now resolve to this machine.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy("setup", false);
    }
  }

  /**
   * Called from the Play button before a project starts. Checks routing
   * readiness; if it isn't set up, runs the one-prompt setup *once* per
   * session. Never throws and never blocks the project from starting — a
   * project still works via localhost even if the user skips DNS setup.
   */
  async function ensureReady(): Promise<void> {
    if (!browser) return;
    try {
      const pf = await safeInvoke<DnsPreflight>("dns_preflight");
      preflight = pf;
      if (pf.ready || autoSetupTried) return;
      autoSetupTried = true;
      await setupLocalDns();
    } catch {
      // Never block project start on a routing check.
    }
  }

  async function restart(): Promise<void> {
    if (isBusy("restart")) return;
    setBusy("restart", true);
    try {
      await safeInvoke("restart_dnsmasq");
      await refresh();
    } catch {
      /* toast already pushed */
    } finally {
      setBusy("restart", false);
    }
  }

  /**
   * Change the domain suffix. Runs the registry migration (renames every
   * project hostname, drops their HTTPS cert dirs so they reissue), then
   * restarts dnsmasq so its wildcard re-points at the new suffix, and
   * refreshes both the DNS view and the projects list.
   */
  async function setSuffix(newSuffix: string): Promise<DomainMigration | null> {
    if (isBusy("suffix")) return null;
    setBusy("suffix", true);
    try {
      const migration = await safeInvoke<DomainMigration>(
        "update_domain_suffix",
        { domainSuffix: newSuffix },
      );
      // Re-point the dnsmasq wildcard at the migrated suffix.
      await safeInvoke("restart_dnsmasq").catch(() => {});
      errorBus.push({
        code: "DNS_SUFFIX_CHANGED",
        whatHappened: `Domain suffix changed to .${migration.newSuffix}.`,
        whyItMatters:
          migration.changedProjects > 0
            ? `${migration.changedProjects} hostname(s) renamed${
                migration.certDirsRemoved > 0
                  ? ` and ${migration.certDirsRemoved} HTTPS cert(s) will reissue`
                  : ""
              }. If DNS routing was installed, reinstall the resolver for the new suffix.`
            : "Reinstall the resolver for the new suffix if you use DNS routing.",
        whoCausedIt: "system",
        severity: "success",
        actions: [],
      });
      await Promise.all([refresh(), projects.refresh()]);
      return migration;
    } catch {
      return null;
    } finally {
      setBusy("suffix", false);
    }
  }

  return {
    get status() {
      return status;
    },
    get settings() {
      return settings;
    },
    get records() {
      return records;
    },
    get hosts() {
      return hosts;
    },
    get preflight() {
      return preflight;
    },
    get loading() {
      return loading;
    },
    get dnsRouting() {
      return status?.installed === true;
    },
    isBusy,
    refresh,
    saveSettings,
    installResolver,
    uninstallResolver,
    restart,
    setSuffix,
    setupLocalDns,
    ensureReady,
  };
}

export const dns = createDnsStore();
