/**
 * PortBay Pro entitlement types — mirror the Rust `crate::entitlements`
 * serialization, which itself mirrors the signed §6 contract in
 * `docs/pro/entitlements.md` (v2, 3-tier). The Rust side is the source of truth.
 */

export type EntitlementState = "anonymous" | "free" | "pro" | "pro-grace" | "unknown-offline";

export type Tier = "anonymous" | "free" | "pro";

export interface Account {
  /** `null` for email-auth accounts. */
  github_id: number | null;
  login: string;
  /** User-set display name (schema ≥ 3); source of the avatar initials. */
  display_name?: string | null;
  /** Server-resolved avatar to fetch — custom upload or GitHub (schema ≥ 3). */
  avatar_url?: string | null;
}

export interface Entitlements {
  /** `null` = unlimited (pro); free = 6; anonymous = 3. */
  max_projects: number | null;
  /** License activation cap (schema ≥ 3): Pro = 2, free/anonymous = 1.
   *  Absent (`undefined`) on a legacy schema-2 cached doc. */
  max_devices?: number | null;
  sync: boolean;
  custom_port_cors: boolean;
  mail: "limited" | "full";
  early_access: boolean;
  priority_support: boolean;
}

export interface EffectiveEntitlement {
  state: EntitlementState;
  tier: Tier;
  entitlements: Entitlements;
  account: Account | null;
}

/** Pro-gated capabilities the gates check via `entitlements.allows(...)`. */
export type GatedFeature =
  | "sync"
  | "custom_port_cors"
  | "mail_full"
  | "early_access"
  | "unlimited_projects";

/** Which upgrade prompt a project-cap block should offer. */
export type UpgradePrompt = "signup" | "pro";

/** Built-in anonymous default — the state before any sign-in. Cap 3, unsigned. */
export const ANONYMOUS_FALLBACK: EffectiveEntitlement = {
  state: "anonymous",
  tier: "anonymous",
  account: null,
  entitlements: {
    max_projects: 3,
    max_devices: 1,
    sync: false,
    custom_port_cors: false,
    mail: "limited",
    early_access: false,
    priority_support: false,
  },
};
