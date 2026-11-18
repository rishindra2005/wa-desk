<script lang="ts">
  import NavRail from "./lib/shell/NavRail.svelte";
  import TopBar from "./lib/shell/TopBar.svelte";
  import CommandPalette from "./lib/shell/CommandPalette.svelte";
  import Toasts from "./lib/components/Toasts.svelte";
  import ContextMenu from "./lib/components/ContextMenu.svelte";
  import ProfileDrawer from "./lib/components/ProfileDrawer.svelte";
  import ExportDialog from "./lib/components/ExportDialog.svelte";
  import ForwardDialog from "./lib/components/ForwardDialog.svelte";
  import { warmPdfjs } from "./lib/components/pdfjs";
  import { ctxMenu } from "./lib/state/menu.svelte";
  import ErrorPanel from "./lib/components/ErrorPanel.svelte";
  import { overlayCount } from "./lib/components/overlay";
  import ChatsView from "./lib/views/chats/ChatsView.svelte";
  import CallsView from "./lib/views/calls/CallsView.svelte";
  import IncomingCallBanner from "./lib/components/IncomingCallBanner.svelte";
  import SearchView from "./lib/views/search/SearchView.svelte";
  import MediaView from "./lib/views/media/MediaView.svelte";
  import DocsView from "./lib/views/docs/DocsView.svelte";
  import ToolsView from "./lib/views/tools/ToolsView.svelte";
  import AutomationsView from "./lib/views/automations/AutomationsView.svelte";
  import SystemView from "./lib/views/system/SystemView.svelte";
  import SettingsView from "./lib/views/settings/SettingsView.svelte";
  import { app, VIEWS } from "./lib/state/app.svelte";
  import { toasts } from "./lib/state/toasts.svelte";
  import ZoomIndicator from "./lib/shell/ZoomIndicator.svelte";
  import { zoomActionForKey, zoomActionForWheel } from "./lib/utils/zoom";

  toasts.navigate = (v) => app.go(v);

  $effect(() => {
    void app.start();
  });

  $effect(() => {
    warmPdfjs();
  });

  $effect(() => {
    document.documentElement.classList.toggle("dark", app.dark);
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && app.profileJid && overlayCount() === 0 && !ctxMenu.isOpen) {
      const t = e.target as HTMLElement | null;
      if (!t || (t.tagName !== "INPUT" && t.tagName !== "TEXTAREA")) {
        app.closeProfile();
        return;
      }
    }
    const zoom = zoomActionForKey(e);
    if (zoom) {
      // App zoom owns Ctrl+= / + / - / 0 everywhere (WebKitGTK has no built-in zoom keys).
      e.preventDefault();
      void app.zoomBy(zoom);
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
      e.preventDefault();
      app.paletteOpen = !app.paletteOpen;
      return;
    }
    if (mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "f") {
      if (overlayCount() > 0) return;
      e.preventDefault();
      app.openSearch();
      return;
    }
    if (e.altKey && !mod && /^Digit[1-9]$/.test(e.code)) {
      if (overlayCount() > 0) return;
      const v = VIEWS[Number(e.code.slice(5)) - 1];
      if (v) {
        e.preventDefault();
        app.go(v.id);
      }
    }
  }

  // Ctrl+wheel zooms the app (one step per gesture tick; the image viewer keeps its own wheel zoom).
  let lastWheel = 0;
  $effect(() => {
    const onWheel = (e: WheelEvent) => {
      const dir = zoomActionForWheel(e);
      if (!dir) return;
      if ((e.target as HTMLElement | null)?.closest?.("[data-testid='lightbox']")) return;
      e.preventDefault();
      const now = Date.now();
      if (now - lastWheel < 120) return;
      lastWheel = now;
      void app.zoomBy(dir);
    };
    window.addEventListener("wheel", onWheel, { passive: false });
    return () => window.removeEventListener("wheel", onWheel);
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="flex h-full min-h-0 bg-bg text-fg">
  <NavRail />
  <div class="flex min-w-0 flex-1 flex-col">
    <TopBar />
    <IncomingCallBanner />
    <main class="min-h-0 flex-1" data-testid="view-{app.view}">
      {#key app.view}
        <svelte:boundary onerror={(e) => console.error(e)}>
          {#if app.view === "chats"}<ChatsView />
          {:else if app.view === "calls"}<CallsView />
          {:else if app.view === "search"}<SearchView />
          {:else if app.view === "media"}<MediaView />
          {:else if app.view === "docs"}<DocsView />
          {:else if app.view === "tools"}<ToolsView />
          {:else if app.view === "automations"}<AutomationsView />
          {:else if app.view === "system"}<SystemView />
          {:else}<SettingsView />{/if}
          {#snippet failed(error, reset)}
            <ErrorPanel {error} {reset} title="This view crashed" />
          {/snippet}
        </svelte:boundary>
      {/key}
    </main>
  </div>
  {#if app.profileJid}<ProfileDrawer />{/if}
</div>

{#if app.paletteOpen}<CommandPalette />{/if}
{#if app.exportDialog}<ExportDialog {...app.exportDialog} />{/if}
{#if app.forwardDialog}<ForwardDialog {...app.forwardDialog} />{/if}
<ContextMenu />
<Toasts />
<ZoomIndicator />
