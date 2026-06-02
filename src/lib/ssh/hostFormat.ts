/**
 * Presentation helpers shared by the SSH host table and detail panel: the
 * `user@host` destination, relative/absolute timestamps, the auth summary, and
 * the colour mapping for stage badges + health dots. Kept here so the table and
 * panel render identically and the token choices live in one place.
 */
import type {
  HostTrust,
  ProbeHealth,
  SshConnectionView,
} from "$lib/types/sshConnections";

/** `user@host`, or just the host when no user is set (OpenSSH Host alias). */
export function destination(c: SshConnectionView): string {
  return c.sshUser.trim() ? `${c.sshUser}@${c.sshHost}` : c.sshHost;
}

/** Compact "15m ago" / "3h ago" / "2d ago" relative stamp; "Never" when unset. */
export function relativeTime(secs: number | null): string {
  if (!secs) return "Never";
  const diff = Math.max(0, Math.floor(Date.now() / 1000) - secs);
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86_400) return `${Math.floor(diff / 3600)}h ago`;
  if (diff < 30 * 86_400) return `${Math.floor(diff / 86_400)}d ago`;
  return `${Math.floor(diff / (30 * 86_400))}mo ago`;
}

/** "Today, 10:11 AM" / "May 18, 9:33 AM" — the secondary line under relative time. */
export function absoluteTime(secs: number | null): string {
  if (!secs) return "";
  const d = new Date(secs * 1000);
  const time = d.toLocaleTimeString(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  if (sameDay) return `Today, ${time}`;
  const date = d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  return `${date}, ${time}`;
}

/** "Apr 12, 2024" — the panel's "Created" row. */
export function dateLabel(secs: number | null): string {
  if (!secs) return "—";
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  });
}

/** Primary + secondary auth summary. No key-algorithm guessing — we surface the
 *  saved key file's name rather than a fabricated `ed25519`. */
export function authSummary(c: SshConnectionView): { label: string; detail: string } {
  switch (c.authKind) {
    case "password":
      return { label: "Password", detail: "keychain" };
    case "agent":
      return { label: "SSH Agent", detail: "ssh-agent" };
    case "key":
    default: {
      const file = (c.keyPath ?? "").split("/").pop() ?? "";
      return { label: "SSH Key", detail: file };
    }
  }
}

/** Icon name for the auth method (from the app's Icon registry). */
export function authIcon(c: SshConnectionView): "key" | "lock" | "users" {
  if (c.authKind === "password") return "lock";
  if (c.authKind === "agent") return "users";
  return "key";
}

/** Known deployment tiers, in display order, for the form + filter tabs. */
export const STAGES = ["production", "staging", "research", "sandbox"] as const;
export type Stage = (typeof STAGES)[number];

/** Title-cased badge text + Tailwind classes for a stage chip. */
export function stageMeta(stage: string | null): {
  label: string;
  chipClass: string;
} | null {
  switch ((stage ?? "").toLowerCase()) {
    case "production":
      return { label: "Production", chipClass: "bg-status-running/12 text-status-running" };
    case "staging":
      return { label: "Staging", chipClass: "bg-status-unhealthy/15 text-status-unhealthy" };
    case "research":
      return { label: "Research", chipClass: "bg-status-starting/12 text-status-starting" };
    case "sandbox":
      return { label: "Sandbox", chipClass: "bg-surface-2 text-fg-muted" };
    default:
      return null;
  }
}

/** Label + dot colour for a probe health state. */
export function healthMeta(health: ProbeHealth | null | undefined): {
  label: string;
  dotClass: string;
} {
  switch (health) {
    case "healthy":
      return { label: "Healthy", dotClass: "bg-status-running" };
    case "degraded":
      return { label: "Degraded", dotClass: "bg-status-unhealthy" };
    case "down":
      return { label: "Down", dotClass: "bg-status-crashed" };
    default:
      return { label: "Unknown", dotClass: "bg-status-stopped" };
  }
}

/** Label + tone for a host-key trust state. */
export function trustMeta(trust: HostTrust | null | undefined): {
  label: string;
  description: string;
  tone: "ok" | "warn" | "danger" | "neutral";
} {
  switch (trust) {
    case "trusted":
      return {
        label: "Trusted",
        description: "You have successfully connected to this host.",
        tone: "ok",
      };
    case "new":
      return {
        label: "First contact",
        description: "Not yet in known_hosts — its key is recorded on first connect.",
        tone: "neutral",
      };
    case "changed":
      return {
        label: "Key changed",
        description: "The host key differs from the one on record. Verify before connecting.",
        tone: "danger",
      };
    default:
      return {
        label: "Unknown",
        description: "Trust couldn't be determined yet — refresh to probe the host.",
        tone: "neutral",
      };
  }
}
