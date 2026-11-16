<script lang="ts">
  import { MessageCircle, Search, Images, FileText, Wrench, Bot, Activity, Settings, Phone } from "@lucide/svelte";
  import { app, VIEWS } from "../state/app.svelte";
  import { calls } from "../state/calls.svelte";
  import type { ViewId } from "../errors";

  const ICONS: Record<ViewId, typeof MessageCircle> = {
    chats: MessageCircle,
    calls: Phone,
    search: Search,
    media: Images,
    docs: FileText,
    tools: Wrench,
    automations: Bot,
    system: Activity,
    settings: Settings,
  };

  function badge(id: ViewId): { n: number; tone: string } | null {
    if (id === "chats" && app.unreadChats > 0) return { n: app.unreadChats, tone: "bg-accent text-accent-fg" };
    if (id === "calls" && calls.missedCount > 0) return { n: calls.missedCount, tone: "bg-danger text-white" };
    if (id === "system" && app.openIssues > 0) return { n: app.openIssues, tone: "bg-danger text-white" };
    if (id === "system" && app.runningJobs.length > 0) return { n: app.runningJobs.length, tone: "bg-info text-white" };
    return null;
  }
</script>

<nav class="flex w-16 shrink-0 flex-col items-center gap-1 border-r border-border bg-surface py-3" aria-label="Main">
  <div class="mb-3 flex h-9 w-9 items-center justify-center rounded-[10px] bg-accent text-[15px] font-bold text-accent-fg" aria-hidden="true">wa</div>
  {#each VIEWS as v, i (v.id)}
    {@const Icon = ICONS[v.id]}
    {@const b = badge(v.id)}
    <button
      class="group relative flex h-10 w-10 items-center justify-center rounded-[var(--radius)] transition-colors {app.view === v.id ? 'bg-accent-soft text-accent-text' : 'text-muted hover:bg-surface-2 hover:text-fg'} {v.id === 'settings' ? 'mt-auto' : ''}"
      aria-label={v.label}
      aria-current={app.view === v.id ? "page" : undefined}
      aria-keyshortcuts="Alt+{i + 1}"
      onclick={() => app.go(v.id)}
      data-testid="nav-{v.id}"
    >
      <Icon size={20} />
      {#if b}
        <span class="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-bold {b.tone}" data-testid="nav-badge-{v.id}">{b.n > 99 ? "99+" : b.n}</span>
      {/if}
      <span
        class="pointer-events-none absolute left-full z-50 ml-2 whitespace-nowrap rounded-md bg-fg px-2 py-1 text-xs font-medium text-bg opacity-0 shadow-[var(--shadow-lg)] transition-opacity delay-300 group-hover:opacity-100 group-focus-visible:opacity-100"
        role="tooltip">{v.label} <span class="opacity-60">Alt+{i + 1}</span></span
      >
    </button>
  {/each}
</nav>
