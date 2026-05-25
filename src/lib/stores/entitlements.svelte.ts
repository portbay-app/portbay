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
import {
  ANONYMOUS_FALLBACK,
  type Account,
  type EffectiveEntitlement,
  type EntitlementState,
  type GatedFeature,
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
    get account(): Account | null {
      return value.account;
    },
    load,
    refresh,
    resync,
    login,
    cancelLogin,
    logout: clear,
    clear,
    allows,
    canAddProject,
    canSandbox,
    maxSandboxProjects,
    upgradePromptAt,
  };
}

export const entitlements = createEntitlementsStore();
