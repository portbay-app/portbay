/**
 * Canonical Pro feature matrix — the single source for "what does Community
 * get vs Pro?". The Pro License surface (About-License dialog), the sign-in /
 * upgrade sheet, and the docs Pro page all read this, so the value list can
 * never drift from what's actually gated.
 *
 * Mirrors the signed entitlement contract (`docs/pro/entitlements.md` §6) and
 * the Rust `entitlements::Entitlements` shape.
 */

import type { IconName } from "$lib/components/atoms/Icon.svelte";

export interface ProFeature {
  /** Stable key (matches a `GatedFeature` where one exists). */
  key: string;
  icon: IconName;
  label: string;
  /** What the free tiers (anonymous / free) get for this row. */
  community: string;
  /** What Pro unlocks. */
  pro: string;
  /** Short phrasing for the upgrade upsell list. */
  perk: string;
}

export const PRO_PRICE = { amount: 59, currency: "USD", interval: "year", label: "$59/yr" };
export const PRO_DEVICES = 2;
/** Pro marketing / pricing page — the "Learn more about Pro" link target. */
export const PRICING_URL = "https://portbay.app/pro";

export const PRO_FEATURES: ProFeature[] = [
  {
    key: "devices",
    icon: "users",
    label: "Devices",
    community: "1",
    pro: "Up to 2",
    perk: "Use Pro on 2 devices",
  },
  {
    key: "projects",
    icon: "layers",
    label: "Projects",
    community: "Up to 6",
    pro: "Unlimited",
    perk: "Unlimited projects",
  },
  {
    key: "sync",
    icon: "cloud",
    label: "Multi-device sync",
    community: "—",
    pro: "End-to-end encrypted",
    perk: "Multi-device sync",
  },
  {
    key: "custom_port_cors",
    icon: "settings",
    label: "Custom ports & CORS",
    community: "Defaults",
    pro: "Fully configurable",
    perk: "Custom ports & CORS",
  },
  {
    key: "mail",
    icon: "server",
    label: "Mail server",
    community: "Catch & view",
    pro: "Full SMTP access",
    perk: "Full mail server",
  },
  {
    key: "early_access",
    icon: "rocket",
    label: "Early access",
    community: "—",
    pro: "New features first",
    perk: "Early access to new features",
  },
  {
    key: "priority_support",
    icon: "users",
    label: "Support",
    community: "Community",
    pro: "Priority",
    perk: "Priority support",
  },
];

/** Icon + label perks for the upgrade upsell (derived from the matrix). */
export const PRO_PERKS = PRO_FEATURES.map((f) => ({ icon: f.icon, label: f.perk }));

/** Where to send the two honest acquisition paths. */
export const DONATE_URL = "https://buymeacoffee.com/beiruti";
export const CONTRIBUTE_URL = "https://github.com/portbay-app/portbay/contribute";

/** Published legal docs (linked from the About-License view). */
export const PRIVACY_URL = "https://docs.portbay.app/legal/privacy";
export const TERMS_URL = "https://docs.portbay.app/legal/terms";
export const LICENSE_URL = "https://github.com/portbay-app/portbay/blob/main/LICENSE";
