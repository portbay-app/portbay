/**
 * account — drives the single <SignInSheet> mounted at the layout root.
 *
 * The sheet is the one surface for signing in / up and for upgrading to Pro.
 * Gates open it with an `intent` so the copy fits the moment:
 *   - "signin" — user chose Sign in from the menu.
 *   - "signup" — hit the anonymous 3-project cap (free unlocks 6).
 *   - "pro"    — hit the free 6-project cap, or chose Upgrade (Pro = unlimited).
 *
 * `reason` is an optional one-line context line shown under the heading.
 */

export type AuthIntent = "signin" | "signup" | "pro";

interface AccountSheetState {
  open: boolean;
  intent: AuthIntent;
  reason?: string;
}

function createAccountStore() {
  let state = $state<AccountSheetState>({ open: false, intent: "signin" });

  return {
    get isOpen() {
      return state.open;
    },
    get intent() {
      return state.intent;
    },
    get reason() {
      return state.reason;
    },
    /** Open the sheet. */
    open(opts: { intent: AuthIntent; reason?: string } = { intent: "signin" }) {
      state = { open: true, intent: opts.intent, reason: opts.reason };
    },
    close() {
      state = { ...state, open: false };
    },
  };
}

export const account = createAccountStore();
