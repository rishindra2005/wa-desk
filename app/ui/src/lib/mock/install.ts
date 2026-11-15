// Installs the mock backend behind Tauri's IPC using `mockIPC`. Events are
// implemented here (instead of `shouldMockEvents`) so that `listen()` /
// unlisten / emit behave exactly like the real event plugin: the backend's
// emissions run the very callbacks that `listen()` registered.

import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { EventMap, EventName } from "../api";
import { MockBackend, MockFail, type MockOptions } from "./backend";

interface Internals {
  runCallback: (id: number, data: unknown) => void;
}

declare global {
  interface Window {
    __waMock?: MockBackend;
  }
}

/** Install the mock IPC layer; returns the backend for inspection. */
export function installMock(opts: MockOptions = {}): MockBackend {
  const backend = new MockBackend(opts);
  const listeners = new Map<string, number[]>();

  const dispatch = (event: string, payload: unknown) => {
    const internals = (window as unknown as { __TAURI_INTERNALS__: Internals }).__TAURI_INTERNALS__;
    for (const handler of [...(listeners.get(event) ?? [])]) {
      internals.runCallback(handler, { event, id: handler, payload });
    }
  };

  mockWindows("main");
  mockIPC(async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    switch (cmd) {
      case "plugin:event|listen": {
        const { event, handler } = args as { event: string; handler: number };
        const list = listeners.get(event) ?? [];
        list.push(handler);
        listeners.set(event, list);
        return handler;
      }
      case "plugin:event|unlisten": {
        const { event, eventId } = args as { event: string; eventId: number };
        listeners.set(event, (listeners.get(event) ?? []).filter((h) => h !== eventId));
        return null;
      }
      case "plugin:event|emit": {
        const { event, payload: p } = args as { event: string; payload: unknown };
        dispatch(event, p);
        return null;
      }
    }
    try {
      // Clone in and out to mimic JSON serialisation across the IPC boundary.
      const cloned = JSON.parse(JSON.stringify(args)) as Record<string, unknown>;
      const result = await backend.handle(cmd, cloned);
      return result === undefined ? null : JSON.parse(JSON.stringify(result));
    } catch (e) {
      if (e instanceof MockFail) throw e.err;
      throw { kind: "internal", message: e instanceof Error ? e.message : String(e) };
    }
  });

  backend.start(<K extends EventName>(name: K, payload: EventMap[K]) => dispatch(name, payload));
  window.__waMock = backend;
  return backend;
}

/** True when running outside Tauri (browser dev, tests). */
export function needsMock(): boolean {
  return typeof window !== "undefined" && !("__TAURI_INTERNALS__" in window);
}
