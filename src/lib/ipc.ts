/**
 * Typed wrappers around the Tauri commands and events.
 *
 * Everything the webview can ask the backend to do goes through here, so the
 * command names live in exactly one place.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  Bootstrap,
  Config,
  HudState,
  MonitorInfo,
  ProviderId,
  Telemetry,
} from "../types";

export const TELEMETRY_EVENT = "codenotch://telemetry";
export const CONFIG_EVENT = "codenotch://config";
export const HUD_STATE_EVENT = "codenotch://hud-state";

/** True when running inside the Tauri shell rather than a bare browser. */
export const IN_TAURI =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/**
 * Call a command, swallowing failures.
 *
 * A HUD that throws a dialog because a window nudge failed would be worse than
 * one that quietly carries on, so IPC errors are logged and dropped.
 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  if (!IN_TAURI) return null;
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    console.warn(`[codenotch] ${command} failed:`, error);
    return null;
  }
}

export const ipc = {
  ready: () => call<Bootstrap>("hud_ready"),
  hover: (hovering: boolean) => call<void>("hud_hover", { hovering }),
  setContentHeight: (height: number) =>
    call<void>("hud_set_content_height", { height }),
  togglePin: () => call<boolean>("hud_toggle_pin"),
  getConfig: () => call<Config>("get_config"),
  setConfig: (config: Config) => call<Config>("set_config", { config }),
  refreshNow: () => call<Telemetry>("refresh_now"),
  focusProvider: (provider: ProviderId, titleHint?: string | null) =>
    call<boolean>("focus_provider", { provider, titleHint: titleHint ?? null }),
  setHidden: (hidden: boolean) => call<void>("set_hidden", { hidden }),
  peek: (seconds?: number) => call<void>("peek", { seconds: seconds ?? null }),
  listMonitors: () => call<MonitorInfo[]>("list_monitors"),
  openConfigDir: () => call<void>("open_config_dir"),
  quit: () => call<void>("quit_app"),
};

/** Subscribe to a backend event; returns an unsubscribe function. */
export function subscribe<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (!IN_TAURI) return Promise.resolve(() => {});
  return listen<T>(event, (e) => handler(e.payload));
}

export const events = {
  telemetry: (handler: (t: Telemetry) => void) =>
    subscribe<Telemetry>(TELEMETRY_EVENT, handler),
  config: (handler: (c: Config) => void) => subscribe<Config>(CONFIG_EVENT, handler),
  hudState: (handler: (s: HudState) => void) =>
    subscribe<HudState>(HUD_STATE_EVENT, handler),
};
