/** Dev-only alias target for `@tauri-apps/api/event`. See `mockBackend.ts`. */
import { mockListen } from "./mockBackend";
import type { AppSnapshot } from "../lib/types";

export type UnlistenFn = () => void;

export function listen<T>(
  _event: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  const stop = mockListen((payload) => handler({ payload } as { payload: T }));
  return Promise.resolve(stop as UnlistenFn);
}

export type { AppSnapshot };
