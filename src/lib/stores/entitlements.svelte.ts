/**
 * Pro entitlement store — the single frontend read for "what is this user
 * entitled to?". Backed by the Rust `entitlements` module (signature
 * verification + offline grace live there); this store just mirrors the
 * computed effective entitlement and exposes gate helpers.
 *
 * Every gate (project cap, custom port/CORS, mail) reads from here — gates
 * never call the backend directly.
 */

import { browser } from "$app/environment";
import { openUrl } from "$lib/security/openUrl";
import { invokeQuiet, safeInvoke } from "$lib/ipc";
import { invalidateUserAvatar } from "$lib/userAvatar";
import {
  ANONYMOUS_FALLBACK,
  type Account,
  type BillingPortalUrls,
  type EffectiveEntitlement,
  type EntitlementState,
  type GatedFeature,
  type SubscriptionStatus,
  type Tier,
  type UpgradePrompt,
} from "$lib/types/entitlements";

/** Login method the sign-in sheet offers. */
export type LoginMethod = "github" | "email";

/** Terminal result of a {@link login} attempt. */
export type LoginResult = "ready" | "expired" | "timeout" | "error";

interface LoginPollResult {
  status: "pending" | "ready" | "expired";
  entitlement?: EffectiveEntitlement;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

/**
 * Community Sandboxed Run allowance (anonymous + free tiers). Pro is unlimited.
 * Mirror of the Rust `entitlements::SANDBOX_COMMUNITY_CAP` — keep the two in sync.
 */
const SANDBOX_COMMUNITY_CAP = 2;

function createEntitlementsStore() {
  let value = $state<EffectiveEntitlement>(ANONYMOUS_FALLBACK);
  let loaded = $state<boolean>(false);
  let loggingIn = $state<boolean>(false);
  let abortLogin = false;

  /** Load the cached effective entitlement (no network). Call on app start. */
  async function load(): Promise<void> {
    if (!browser) return;
    try {
      value = await invokeQuiet<EffectiveEntitlement>("get_entitlement");
    } catch {
      value = ANONYMOUS_FALLBACK; // offline / no cache → anonymous, silently
    } finally {
      loaded = true;
    }
  }

  /** Re-fetch + verify from the issuer using a session token (after login). */
  async function refresh(token: string): Promise<void> {
    value = await safeInvoke<EffectiveEntitlement>("refresh_entitlement", { token });
  }

  /**
   * Re-verify a stored session on app start: rotates the refresh token, fetches
   * a fresh signed entitlement, and updates the effective state. No-op (stays
   * on the cached/anonymous value) when not signed in or offline. Quiet — never
   * toasts; failures degrade to the cached entitlement.
   */
  async function resync(): Promise<void> {
    if (!browser) return;
    try {
      value = await invokeQuiet<EffectiveEntitlement>("account_resync");
    } catch {
      /* keep current value (offline / transient) */
    }
  }

  /**
   * Drive a full login: open the flow, launch the browser (GitHub) or trigger
   * the magic-link email, then poll until the session is issued. On success the
   * store reflects the new (signed-in) entitlement. Cancellable via
   * {@link cancelLogin}.
   */
  async function login(
    method: LoginMethod,
    email?: string,
    opts: { timeoutMs?: number; intervalMs?: number } = {},
  ): Promise<LoginResult> {
    const timeoutMs = opts.timeoutMs ?? 5 * 60_000;
    const intervalMs = opts.intervalMs ?? 2000;
    abortLogin = false;
    loggingIn = true;
    try {
      let begun: { authorize_url: string | null };
      try {
        begun = await safeInvoke<{ authorize_url: string | null }>("begin_login", {
          method,
          email: email ?? null,
        });
      } catch {
        return "error";
      }
      if (begun.authorize_url) {
        try {
          await openUrl(begun.authorize_url);
        } catch {
          /* opener toasts; keep polling in case the user opens it manually */
        }
      }

      const deadline = Date.now() + timeoutMs;
      while (!abortLogin && Date.now() < deadline) {
        await sleep(intervalMs);
        if (abortLogin) break;
        let poll: LoginPollResult;
        try {
          poll = await invokeQuiet<LoginPollResult>("poll_login");
        } catch {
          continue; // transient network hiccup — keep polling
        }
        if (poll.status === "ready" && poll.entitlement) {
          value = poll.entitlement;
          return "ready";
        }
        if (poll.status === "expired") return "expired";
      }
      void invokeQuiet("cancel_login").catch(() => {});
      return abortLogin ? "expired" : "timeout";
    } finally {
      loggingIn = false;
    }
  }

  /** Abort an in-flight {@link login} (e.g. the sign-in sheet was closed). */
  function cancelLogin(): void {
    abortLogin = true;
    void invokeQuiet("cancel_login").catch(() => {});
  }

  /** Sign out: revoke server-side, clear the keychain + cache, drop to anonymous. */
  async function clear(): Promise<void> {
    try {
      value = await safeInvoke<EffectiveEntitlement>("logout");
    } catch {
      value = ANONYMOUS_FALLBACK;
    }
    // Drop the cached avatar promise so a subsequent sign-in re-fetches rather
    // than reusing the prior account's face (the backend clears its disk cache).
    invalidateUserAvatar();
  }

  /**
   * Permanently delete the account (GDPR erasure). Server-first: the backend
   * only clears local state after the cloud confirms the deletion, so a
   * failure (toasted by `safeInvoke`) leaves the user signed in. On success
   * the store drops to anonymous.
   */
  async function deleteAccount(): Promise<void> {
    value = await safeInvoke<EffectiveEntitlement>("delete_account");
    invalidateUserAvatar();
  }

  /**
   * Export the account's hosted data (GDPR portability) as JSON to
   * `destPath`, a location the user picked via the save dialog.
   */
  async function exportAccountData(destPath: string): Promise<void> {
    await safeInvoke<void>("export_account_data", { destPath });
  }

  /**
   * Set or clear the account display name (Pro/Free signed-in only). Re-pulls
   * the freshly signed entitlement, so `value.account.display_name` updates.
   */
  async function updateDisplayName(name: string | null): Promise<void> {
    value = await safeInvoke<EffectiveEntitlement>("update_display_name", { name });
    invalidateUserAvatar();
  }

  /** Upload a custom avatar from a local file path. Re-pulls the entitlement. */
  async function uploadAvatar(path: string): Promise<void> {
    value = await safeInvoke<EffectiveEntitlement>("upload_avatar", { path });
    invalidateUserAvatar();
  }

  /** Remove the custom avatar (revert to GitHub/initials). Re-pulls the entitlement. */
  async function removeAvatar(): Promise<void> {
    value = await safeInvoke<EffectiveEntitlement>("remove_avatar");
    invalidateUserAvatar();
  }

  /**
   * Start the Pro purchase: ask the backend for a per-user Paddle checkout URL
   * and open it in the system browser. Requires a signed-in session (the URL is
   * attributed to the account so the webhook can issue Pro). `safeInvoke` toasts
   * a friendly error if not signed in or checkout isn't configured yet.
   */
  async function startCheckout(): Promise<void> {
    const url = await safeInvoke<string>("pro_checkout_url");
    await openUrl(url);
  }

  /**
   * Fetch the account's subscription state from the issuer (renewal date,
   * scheduled cancellation, past-due). Quiet: `null` on no subscription
   * (contribution-Pro) *and* on transient failures — the billing block simply
   * doesn't render a status line it can't vouch for.
   */
  async function fetchSubscription(): Promise<SubscriptionStatus | null> {
    if (!browser) return null;
    try {
      return await invokeQuiet<SubscriptionStatus | null>("subscription_status");
    } catch {
      return null;
    }
  }

  /**
   * Open the Paddle customer portal (payment method, invoices, cancel) in the
   * system browser. `kind` picks the deep link; missing deep links fall back
   * to the portal overview. URLs are short-lived and fetched fresh per click.
   * `safeInvoke` toasts when signed out or billing isn't configured.
   */
  async function openBillingPortal(
    kind: "overview" | "cancel" | "payment" = "overview",
  ): Promise<void> {
    const urls = await safeInvoke<BillingPortalUrls>("billing_portal_url");
    const url =
      kind === "cancel"
        ? (urls.cancelUrl ?? urls.overviewUrl)
        : kind === "payment"
          ? (urls.updatePaymentUrl ?? urls.overviewUrl)
          : urls.overviewUrl;
    await openUrl(url);
  }

  /** Whether a Pro-gated feature is currently unlocked. */
  function allows(feature: GatedFeature): boolean {
    const e = value.entitlements;
    switch (feature) {
      case "sync":
        return e.sync;
      case "custom_port_cors":
        return e.custom_port_cors;
      case "mail_full":
        return e.mail === "full";
      case "early_access":
        return e.early_access;
      case "unlimited_projects":
        return e.max_projects === null;
    }
  }

  /** Whether another project may be created given the current count. */
  function canAddProject(currentCount: number): boolean {
    const max = value.entitlements.max_projects;
    return max === null || currentCount < max;
  }

  /**
   * How many projects may have Sandboxed Run enabled at once. `null` =
   * unlimited (Pro). Mirrors the backend `Entitlements::max_sandbox_projects`:
   * Pro (unlimited projects) is uncapped; the community tiers share a small cap
   * so the feature is usable without paying.
   */
  function maxSandboxProjects(): number | null {
    return value.entitlements.max_projects === null ? null : SANDBOX_COMMUNITY_CAP;
  }

  /** Whether another project may be sandboxed given how many already are. */
  function canSandbox(currentCount: number): boolean {
    const max = maxSandboxProjects();
    return max === null || currentCount < max;
  }

  /**
   * Which upgrade prompt to show when a project-cap block is hit:
   * - anonymous at the cap → "signup" (free unlocks 6)
   * - free at the cap      → "pro"    (Pro unlocks unlimited)
   * - otherwise            → null (not at a cap, or already unlimited)
   */
  function upgradePromptAt(currentCount: number): UpgradePrompt | null {
    if (canAddProject(currentCount)) return null;
    return value.tier === "anonymous" ? "signup" : "pro";
  }

  return {
    get value(): EffectiveEntitlement {
      return value;
    },
    get loaded(): boolean {
      return loaded;
    },
    get loggingIn(): boolean {
      return loggingIn;
    },
    get state(): EntitlementState {
      return value.state;
    },
    get tier(): Tier {
      return value.tier;
    },
    get isPro(): boolean {
      return value.state === "pro" || value.state === "pro-grace";
    },
    get isSignedIn(): boolean {
      return value.account !== null;
    },
    /** `null` = unlimited. */
    get maxProjects(): number | null {
      return value.entitlements.max_projects;
    },
    /** License device-activation cap (Pro = 2). Defaults to 2 for a legacy
     *  schema-2 Pro doc that predates the field; 1 otherwise. */
    get maxDevices(): number {
      const m = value.entitlements.max_devices;
      if (typeof m === "number") return m;
      return value.entitlements.sync ? 2 : 1;
    },
    get account(): Account | null {
      return value.account;
    },
    /** Entitlement acquisition source from the signed doc (`"subscription"`,
     *  `"donate"`, …); `null` for synthetic states. */
    get source(): string | null {
      return value.source ?? null;
    },
    /** Whether this Pro came from a paid subscription — i.e. there is billing
     *  to manage. Contribution/donation Pro returns false. */
    get hasManagedBilling(): boolean {
      return (
        (value.state === "pro" || value.state === "pro-grace") &&
        value.source === "subscription"
      );
    },
    load,
    refresh,
    resync,
    updateDisplayName,
    uploadAvatar,
    removeAvatar,
    login,
    cancelLogin,
    logout: clear,
    clear,
    deleteAccount,
    exportAccountData,
    startCheckout,
    fetchSubscription,
    openBillingPortal,
    allows,
    canAddProject,
    canSandbox,
    maxSandboxProjects,
    upgradePromptAt,
  };
}

export const entitlements = createEntitlementsStore();
