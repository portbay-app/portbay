/**
 * Wire shapes for the DNS commands (commands::dnsmasq). Field names follow
 * the Rust `#[serde(rename_all = "camelCase")]` convention.
 */

/** Result of `dnsmasq_resolver_status`. */
export interface ResolverStatus {
  suffix: string;
  installed: boolean;
  path: string;
  currentContents: string | null;
  currentPort: number;
}

/** Editable dnsmasq tuning — mirrors `registry::DnsmasqSettings`. */
export interface DnsmasqSettings {
  cacheSize: number;
  localTtl: number;
  disableNegativeCache: boolean;
}

export type DnsRecordKind = "wildcard" | "project";
export type RoutedVia = "dnsmasq" | "hosts";

/** One row in the DNS records list. */
export interface DnsRecord {
  hostname: string;
  target: string;
  kind: DnsRecordKind;
  projectId: string | null;
  projectName: string | null;
  routedVia: RoutedVia;
}

/** One entry from PortBay's managed `/etc/hosts` block. */
export interface ManagedHostsEntry {
  ip: string;
  hostname: string;
}

/** Result of `dns_preflight` — first-run readiness for local DNS routing. */
export interface DnsPreflight {
  suffix: string;
  dnsmasqPort: number;
  helperInstalled: boolean;
  resolverInstalled: boolean;
  dnsmasqRunning: boolean;
  port80InUse: boolean;
  port443InUse: boolean;
  ready: boolean;
}

/** Result of `update_domain_suffix`. */
export interface DomainMigration {
  oldSuffix: string;
  newSuffix: string;
  changedProjects: number;
  certDirsRemoved: number;
}

export const DEFAULT_DNS_SETTINGS: DnsmasqSettings = {
  cacheSize: 150,
  localTtl: 0,
  disableNegativeCache: false,
};

/** Hard caps mirrored from the Rust side, used to clamp the form inputs. */
export const MAX_DNS_CACHE_SIZE = 10_000;
export const MAX_DNS_LOCAL_TTL = 86_400;
