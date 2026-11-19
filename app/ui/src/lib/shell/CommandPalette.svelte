<script lang="ts">
  import { Search, ArrowRight, Play, MessageCircle, Palette, RefreshCw } from "@lucide/svelte";
  import * as api from "../api";
  import type { ChatSummary } from "../bindings/ChatSummary";
  import type { JobSpec } from "../bindings/JobSpec";
  import Avatar from "../components/Avatar.svelte";
  import { trapFocus } from "../components/overlay";
  import { app, VIEWS } from "../state/app.svelte";
  import { toasts } from "../state/toasts.svelte";
  import { debounce } from "../utils/misc";

  interface Cmd {
    id: string;
    label: string;
    group: "Chats" | "Search" | "Go to" | "Run" | "Theme";
    hint?: string;
    chat?: ChatSummary;
    run: () => void | Promise<void>;
  }

  let q = $state("");
  let chats = $state<ChatSummary[]>([]);
  /** The query `chats` was fetched for; results for an older query are never shown. */
  let chatsFor = $state<string | null>(null);
  let active = $state(0);
  let listEl: HTMLUListElement | undefined = $state();

  const close = () => (app.paletteOpen = false);

  async function job(spec: JobSpec) {
    try {
      const j = await api.startJob(spec);
      app.upsertJob(j);
      toasts.info(`Started: ${j.label}`);
    } catch (e) {
      toasts.error(e, "Couldn't start job");
    }
  }

  const staticCmds: Cmd[] = [
    ...VIEWS.map((v, i) => ({ id: `go:${v.id}`, label: `Open ${v.label}`, group: "Go to" as const, hint: `Alt+${i + 1}`, run: () => app.go(v.id) })),
    { id: "run:backfill", label: "Backfill media (default window)", group: "Run", run: () => job({ type: "backfill", days: app.settings?.backfill_days ?? 30, kinds: ["image", "video", "audio", "document"], workers: 4, retry_failed: false }) },
    { id: "run:reindex", label: "Rebuild search index", group: "Run", run: () => job({ type: "reindex" }) },
    { id: "run:thumbs", label: "Generate thumbnails", group: "Run", run: () => job({ type: "thumbnails" }) },
    { id: "run:link", label: "Link phone media", group: "Run", run: () => job({ type: "link_phone_media", extra_dirs: app.settings?.link_extra_dirs ?? [] }) },
    { id: "run:phone", label: "Phone sync (USB)", group: "Run", run: () => job({ type: "phone_sync" }) },
    { id: "run:reconnect", label: "Reconnect WhatsApp bridge", group: "Run", run: async () => {
      try {
        app.conn = await api.reconnect();
      } catch (e) {
        toasts.error(e);
      }
    } },
    { id: "theme:light", label: "Theme: Light", group: "Theme", run: () => app.setTheme("light") },
    { id: "theme:dark", label: "Theme: Dark", group: "Theme", run: () => app.setTheme("dark") },
    { id: "theme:system", label: "Theme: System", group: "Theme", run: () => app.setTheme("system") },
  ];

  const fetchChats = debounce(async (text: string) => {
    let found: ChatSummary[] = [];
    try {
      const page = await api.listChats({ filter: text || null, kind: "all", unread_only: false, archived: false, cursor: null, limit: text ? 8 : 5 });
      found = page.items;
    } catch {
      found = [];
    }
    // A late answer for an earlier query must not reorder the list under the user.
    if (text !== q.trim()) return;
    chats = found;
    chatsFor = text;
  }, 80);

  $effect(() => {
    fetchChats(q.trim());
    active = 0;
  });

  const items = $derived.by(() => {
    const t = q.trim().toLowerCase();
    const visibleChats = chatsFor === q.trim() ? chats : [];
    const out: Cmd[] = visibleChats.map((c) => ({ id: `chat:${c.jid}`, label: c.name, group: "Chats", chat: c, run: () => app.openChat(c.jid) }));
    const words = t.split(/\s+/).filter(Boolean);
    out.push(...staticCmds.filter((c) => words.every((w) => `${c.group} ${c.label}`.toLowerCase().includes(w))));
    // Full-text search is the fallback, after exact chat/command matches.
    if (t) out.push({ id: "search", label: `Search messages for “${q.trim()}”`, group: "Search", run: () => app.openSearch(q.trim()) });
    return out;
  });

  function exec(c: Cmd | undefined) {
    if (!c) return;
    close();
    void c.run();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") active = Math.min(items.length - 1, active + 1);
    else if (e.key === "ArrowUp") active = Math.max(0, active - 1);
    else if (e.key === "Enter") exec(items[active]);
    else return;
    e.preventDefault();
    queueMicrotask(() => listEl?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" }));
  }

  const ICON = { Chats: MessageCircle, Search, "Go to": ArrowRight, Run: Play, Theme: Palette };
</script>

<div class="fixed inset-0 z-[55] flex items-start justify-center bg-black/35 pt-[12vh] anim-in" role="presentation" onmousedown={(e) => e.target === e.currentTarget && close()}>
  <div class="w-full max-w-xl overflow-hidden rounded-[12px] border border-border bg-surface shadow-[var(--shadow-lg)]" role="dialog" aria-modal="true" aria-label="Command palette" tabindex="-1" use:trapFocus={close} data-testid="palette">
    <div class="flex items-center gap-2 border-b border-border px-4">
      <Search size={16} class="text-faint" />
      <input
        class="h-12 flex-1 bg-transparent text-[15px] outline-none placeholder:text-faint"
        placeholder="Jump to a chat, run a job, open a view…"
        bind:value={q}
        onkeydown={onKey}
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-activedescendant="pal-{active}"
        aria-label="Command"
        data-autofocus
        data-testid="palette-input"
      />
      <span class="kbd">Esc</span>
    </div>
    <ul id="palette-list" bind:this={listEl} role="listbox" class="max-h-[50vh] overflow-y-auto p-1.5">
      {#each items as c, i (c.id)}
        {@const Icon = ICON[c.group]}
        {#if i === 0 || items[i - 1]?.group !== c.group}
          <li class="px-2.5 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wide text-faint" role="presentation">{c.group}</li>
        {/if}
        <li id="pal-{i}" role="option" aria-selected={i === active} data-index={i}>
          <button
            tabindex="-1"
            class="flex w-full items-center gap-2.5 rounded-md px-2.5 py-2 text-left text-[13px] {i === active ? 'bg-accent-soft text-accent-text' : ''}"
            onmouseenter={() => (active = i)}
            onclick={() => exec(c)}
            data-testid="palette-item"
            data-group={c.group}
          >
            {#if c.chat}<Avatar name={c.chat.name} id={c.chat.jid} url={c.chat.avatar_url} type={c.chat.chat_type} size={22} />{:else if c.id === "run:reconnect"}<RefreshCw size={15} />{:else}<Icon size={15} />{/if}
            <span class="min-w-0 flex-1 truncate">{c.label}</span>
            {#if c.hint}<span class="kbd">{c.hint}</span>{/if}
          </button>
        </li>
      {:else}
        <li class="px-3 py-6 text-center text-[13px] text-muted">No matches</li>
      {/each}
    </ul>
  </div>
</div>
