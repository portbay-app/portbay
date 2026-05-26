import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { assertSafeOpenUrl } from "./urlPolicy";

export async function openUrl(raw: string): Promise<void> {
  const url = assertSafeOpenUrl(raw);
  // The hosted web-simulator build (try.portbay.app) has no Tauri runtime, so
  // the opener plugin is a no-op there — links would silently do nothing.
  // Open a normal new tab in the browser; the desktop app keeps using the
  // Tauri opener so the OS default browser handles it.
  if (import.meta.env.PUBLIC_SIMULATOR === "true") {
    window.open(url, "_blank", "noopener,noreferrer");
    return;
  }
  await tauriOpenUrl(url);
}
