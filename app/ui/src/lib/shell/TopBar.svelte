<script lang="ts">
  import { Search, Sun, Moon, Monitor } from "@lucide/svelte";
  import { app, VIEWS } from "../state/app.svelte";
  import { bridgeTone } from "../views/system/bridge";

  const tone = $derived(bridgeTone(app.conn?.state));
  const title = $derived(VIEWS.find((v) => v.id === app.view)?.label ?? "");
  const isMac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform);

  function cycleTheme() {
    const next = app.theme === "system" ? "light" : app.theme === "light" ? "dark" : "system";
    void app.setTheme(next);
  }
</script>

<header class="flex h-12 shrink-0 items-center gap-3 border-b border-border bg-surface px-4">
  <h1 class="w-40 truncate text-[15px] font-semibold">{title}</h1>
  <button
    class="mx-auto flex h-8 w-full max-w-md items-center gap-2 rounded-[var(--radius)] border border-border bg-surface-2 px-3 text-[13px] text-faint transition-colors hover:border-border-strong"
    onclick={() => (app.paletteOpen = true)}
    aria-label="Open command palette"
    aria-keyshortcuts="Control+K"
    data-testid="global-search"
  >
    <Search size={14} />
    <span class="flex-1 text-left">Search or jump to…</span>
    <span class="kbd">{isMac ? "⌘" : "Ctrl"} K</span>
  </button>
  <button
    class="flex h-7 items-center gap-1.5 rounded-full border border-border px-2.5 text-xs font-medium hover:bg-surface-2"
    onclick={() => app.go("system")}
    aria-label="WhatsApp bridge: {tone.label}. Open System"
    data-testid="bridge-pill"
  >
    <span class="relative flex h-2 w-2">
      {#if app.conn?.state === "connecting" || app.conn?.state === "pairing"}<span class="absolute inline-flex h-full w-full animate-ping rounded-full opacity-60" style="background:{tone.color}"></span>{/if}
      <span class="relative inline-flex h-2 w-2 rounded-full" style="background:{tone.color}"></span>
    </span>
    {tone.label}
  </button>
  <button class="icon-btn" onclick={cycleTheme} aria-label="Theme: {app.theme}. Click to change" data-testid="theme-toggle">
    {#if app.theme === "light"}<Sun size={17} />{:else if app.theme === "dark"}<Moon size={17} />{:else}<Monitor size={17} />{/if}
  </button>
</header>
