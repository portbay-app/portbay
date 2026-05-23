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
}
