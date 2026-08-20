/** Dev-only alias target for `@tauri-apps/api/core`. See `mockBackend.ts`. */
import { mockInvoke } from "./mockBackend";

export function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return mockInvoke<T>(command, args);
}
