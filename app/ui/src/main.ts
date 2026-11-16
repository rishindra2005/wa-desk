import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { app } from "./lib/state/app.svelte";

async function boot() {
  // Outside Tauri (browser dev, Playwright) run against the in-memory mock backend.
  if (!("__TAURI_INTERNALS__" in window)) {
    const { installMock } = await import("./lib/mock/install");
    const params = new URLSearchParams(location.search);
    installMock({ quiet: params.has("quiet"), speed: params.has("fast") ? 4 : 1 });
    // Test hook (mock mode only): lets e2e drive navigation such as jumps to old messages.
    (window as unknown as { __waApp?: typeof app }).__waApp = app;
  }
  // Apply the last theme before first paint to avoid a flash.
  try {
    const t = localStorage.getItem("wa-desk.theme");
    const dark = t === "dark" || ((t === null || t === "system") && matchMedia("(prefers-color-scheme: dark)").matches);
    document.documentElement.classList.toggle("dark", dark);
  } catch {
    /* ignore */
  }
  const target = document.getElementById("app");
  if (target) mount(App, { target });
}

void boot();
