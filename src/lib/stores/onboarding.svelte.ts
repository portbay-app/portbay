/**
 * Onboarding state: marker presence, registry emptiness, and helpers for
 * marking / resetting + invoking the scaffolder.
 *
 * The store loads its status once on app boot (from `+layout.svelte`'s
 * onMount) and exposes a `shouldOnboard` derived view so the first-run
 * router can redirect to `/onboarding` without a second roundtrip.
 */
import { safeInvoke } from "$lib/ipc";
import type { OnboardingStatus } from "$lib/types/onboarding";

function createOnboardingStore() {
  let status = $state<OnboardingStatus | null>(null);
  let loading = $state<boolean>(false);

  async function refresh(): Promise<void> {
    loading = true;
    try {
      status = await safeInvoke<OnboardingStatus>("onboarding_status");
    } finally {
      loading = false;
    }
  }

  async function markOnboarded(): Promise<void> {
    await safeInvoke<void>("mark_onboarded");
    if (status) status = { ...status, onboarded: true };
  }

  async function reset(): Promise<void> {
    await safeInvoke<void>("reset_onboarding");
    if (status) status = { ...status, onboarded: false };
  }

  return {
    get value() {
      return status;
    },
    get loading() {
      return loading;
    },
    /**
     * True when the app should route to `/onboarding` on cold boot:
     * marker absent AND registry empty. Both conditions are needed so
     * users who imported via the CLI but never marked don't get
     * trapped in the onboarding loop.
     */
    get shouldOnboard() {
      return status !== null && !status.onboarded && status.registryEmpty;
    },
    refresh,
    markOnboarded,
    reset,
  };
}

export const onboarding = createOnboardingStore();
