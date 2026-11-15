import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

const env = (globalThis as { process?: { env: Record<string, string | undefined> } }).process?.env ?? {};

// Tauri expects a fixed dev port; fail instead of silently picking another.
export default defineConfig({
  plugins: [tailwindcss(), svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true, host: "127.0.0.1" },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: { target: "safari16", outDir: "dist", sourcemap: true, chunkSizeWarningLimit: 2500 },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts"],
    setupFiles: ["src/test-setup.ts"],
  },
  resolve: env.VITEST ? { conditions: ["browser"] } : undefined,
});
