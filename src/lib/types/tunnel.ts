/**
 * Cloudflare Tunnel status. Mirrors
 * `src-tauri/src/tunnel/lifecycle.rs::TunnelStatus`.
 */
export interface TunnelStatus {
  projectId: string;
  upstreamUrl: string;
  publicUrl: string | null;
  running: boolean;
  startedAtMs: number;
  /** `true` for a bring-your-own named tunnel (stable custom hostname). */
  custom?: boolean;
}

/**
 * A custom named Cloudflare tunnel attached to a project. Mirrors
 * `registry::CustomTunnelConfig`.
 */
export interface CustomTunnelConfig {
  tunnelId: string;
  credentialsFile: string;
  hostname: string;
}

/**
 * A named tunnel detected under `~/.cloudflared`. Mirrors
 * `tunnel::named::DetectedTunnel`.
 */
export interface DetectedTunnel {
  uuid: string;
  credentialsFile: string;
  suggestedHostname: string | null;
}
