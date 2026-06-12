/**
 * Once-per-app-session guard for the Accessibility drag-to-grant sheet, shared
 * between the boot-time check (root layout) and the settings panel
 * (DictateAnywhereControls) so the two hosts never stack duplicate dialogs.
 *
 * Module-scope on purpose: one instance per webview session, reset by reload.
 */
export const grantPromptSession = {
  /** True once any host has shown the Accessibility grant sheet this session. */
  shown: false,
};
