import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { assertSafeOpenUrl } from "./urlPolicy";

export async function openUrl(raw: string): Promise<void> {
  await tauriOpenUrl(assertSafeOpenUrl(raw));
}
