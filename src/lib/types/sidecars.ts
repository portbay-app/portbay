/**
 * Sidecar status taxonomy — mirrors `commands::dto::SidecarHealth` on the
 * Rust side. See `src-tauri/src/commands/dto.rs`.
 */

export type SidecarKey =
  | "processCompose"
  | "caddy"
  | "mkcertCa"
  | "dnsmasq"
  | "mailpit"
  | "hostsHelper";

export type SidecarState =
  | "running"
  | "stopped"
  | "not_installed"
  | "unreachable";

export interface SidecarStatus {
  name: string;
  status: SidecarState;
  detail?: string;
  lastError?: string;
}

export interface SidecarHealth {
  processCompose: SidecarStatus;
  caddy: SidecarStatus;
  mkcertCa: SidecarStatus;
  dnsmasq: SidecarStatus;
  mailpit: SidecarStatus;
  hostsHelper: SidecarStatus;
}

/** Human-readable label per sidecar key — what users see on the card. */
export const sidecarTitle: Record<SidecarKey, string> = {
  processCompose: "Process Compose",
  caddy: "Caddy",
  mkcertCa: "mkcert CA",
  dnsmasq: "dnsmasq",
  mailpit: "Mailpit",
  hostsHelper: "/etc/hosts",
};

/** Order in which sidecars appear in the dashboard row. */
export const SIDECAR_ORDER: SidecarKey[] = [
  "processCompose",
  "caddy",
  "mkcertCa",
  "dnsmasq",
  "mailpit",
  "hostsHelper",
];
