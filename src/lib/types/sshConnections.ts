import type { SshAuthKind } from "$lib/types/sshTunnels";

/** Forward-proxy protocol fronting a connection's first transport hop. */
export type SshProxyKind = "socks5" | "http";

/**
 * A forward proxy (SOCKS5 / HTTP CONNECT) dialled before the SSH target. The
 * proxy password is never carried in views — it lives in the OS keychain and is
 * set via {@link SaveSshConnectionInput.proxyPassword}.
 */
export interface SshProxyConfig {
  kind: SshProxyKind;
  host: string;
  port: number;
  /** Proxy auth username; omit/null for an open proxy. */
  username?: string | null;
}

/** A saved SSH connection plus the two facts the dashboard derives per host. */
export interface SshConnectionView {
  id: string;
  name: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath: string | null;
  proxyJump: string | null;
  /** Reusable identity this host borrows user / key / auth from, if any. */
  identityId: string | null;
  /** Forward proxy dialled before the target, if configured. */
  proxy: SshProxyConfig | null;
  /** Free-form labels for grouping/filtering. */
  tags: string[];
  /** CSS colour (hex or token) for the host dot, when set. */
  color: string | null;
  notes: string | null;
  /** Cached `uname`/os-release string, refreshed on demand. */
  detectedOs: string | null;
  /** Environment id driving the brand mark (`cpanel`, `ubuntu`, `aws`, …). */
  environment: string | null;
  /** Deployment tier shown in the Environment column (`production` / `staging` / `research` / `sandbox`). */
  stage: string | null;
  /** Provider region label (`us-east-1`, `nyc3`, …). */
  region: string | null;
  /** Detected cloud provider (`aws`, `digitalocean`, …), distinct from the
   *  `environment` brand mark (which may be a control panel or distro). */
  provider: string | null;
  /** Epoch seconds when the host was first saved (drives the "Created" row). */
  createdAt: number | null;
  /** Epoch seconds of last successful use (drives ordering). */
  lastUsed: number | null;
  /** How many saved tunnels ride on this host. */
  tunnelCount: number;
  /** Whether any tunnel references it (delete is blocked while true). */
  inUse: boolean;
}

/** One importable host parsed from `~/.ssh/config` (preview only). */
export interface SshConfigCandidate {
  /** The `Host` alias — the suggested connection name. */
  hostAlias: string;
  /** `HostName`, falling back to the alias when omitted. */
  sshHost: string;
  sshPort: number;
  sshUser: string;
  /** `IdentityFile` (raw; `~` preserved). */
  keyPath: string | null;
  proxyJump: string | null;
  /** A wildcard/negation `Host` pattern (`*`, `?`, `!`) — flagged, not importable. */
  wildcard: boolean;
  /** A saved connection already uses this alias's id; importing makes a duplicate (never an overwrite). */
  alreadyExists: boolean;
}

export interface SaveSshConnectionInput {
  /** Existing id to update; omit/blank to create. */
  id?: string | null;
  name: string;
  sshHost: string;
  sshPort: number;
  sshUser: string;
  authKind: SshAuthKind;
  keyPath?: string | null;
  proxyJump?: string | null;
  identityId?: string | null;
  /** Forward proxy (SOCKS5 / HTTP CONNECT). Omit/null for a direct connection. */
  proxy?: SshProxyConfig | null;
  tags: string[];
  color?: string | null;
  notes?: string | null;
  /** Manual environment override (`cpanel`, `ubuntu`, `aws`, …). Blank/`auto` = let detection decide. */
  environment?: string | null;
  /** Deployment tier (`production` / `staging` / `research` / `sandbox`). Blank = none. */
  stage?: string | null;
  /** Provider region label (`us-east-1`, `nyc3`, …). Blank = none. */
  region?: string | null;
  /** Password to store in the keychain (password auth only). Blank leaves any existing one. */
  password?: string | null;
  /** Proxy password to store in the keychain (authenticated proxy only). Blank leaves any existing one. */
  proxyPassword?: string | null;
}

/** Coarse reachability banding from a host probe. */
export type ProbeHealth = "healthy" | "degraded" | "down" | "unknown";

/** Host-key trust state against the local `known_hosts`. */
export type HostTrust = "trusted" | "new" | "changed" | "unknown";

/**
 * Result of a single unauthenticated probe handshake — the data behind the
 * dashboard's Health column and the panel's fingerprint / Host Trust card.
 */
export interface ProbeResult {
  /** TCP port answered (the SSH handshake may still have failed). */
  reachable: boolean;
  /** Round-trip to a completed handshake, in milliseconds. */
  latencyMs: number | null;
  health: ProbeHealth;
  /** `SHA256:<base64>` of the server's host key, when captured. */
  fingerprint: string | null;
  trust: HostTrust;
}
