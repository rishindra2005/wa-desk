<!-- Searchable chat picker (multi- or single-select) backed by list_chats. -->
<script lang="ts">
  import { Check, ChevronDown, X } from "@lucide/svelte";
  import * as api from "../api";
  import type { ChatSummary } from "../bindings/ChatSummary";
  import type { ChatKind } from "../bindings/ChatKind";
  import Avatar from "./Avatar.svelte";
  import { debounce } from "../utils/misc";

  interface Props {
    selected: string[];
    multiple?: boolean;
    label: string;
    placeholder?: string;
    kind?: ChatKind;
    onchange?: (jids: string[]) => void;
    testid?: string;
  }
  let { selected = $bindable(), multiple = true, label, placeholder = "Any chat", kind = "all", onchange, testid }: Props = $props();

  const names = new Map<string, string>();
  let namesVersion = $state(0);
  let open = $state(false);
  let query = $state("");
  let results = $state<ChatSummary[]>([]);
  let active = $state(0);
  let root: HTMLDivElement | undefined = $state();
  const listId = `cp${Math.random().toString(36).slice(2, 8)}`;

  async function fetch(q: string) {
    try {
      const page = await api.listChats({ filter: q || null, kind, unread_only: false, cursor: null, limit: 50 });
      results = page.items;
      for (const c of page.items) names.set(c.jid, c.name);
      namesVersion++;
      active = 0;
    } catch {
      results = [];
    }
  }
  const search = debounce((q: string) => void fetch(q), 150);

  $effect(() => {
    // Resolve names for preselected chats.
    for (const jid of selected) {
      if (!names.has(jid)) {
        names.set(jid, jid);
        api.getChat(jid).then((c) => {
          names.set(jid, c.name);
          namesVersion++;
        }, () => {});
      }
    }
  });

  function nameOf(jid: string): string {
    void namesVersion;
    return names.get(jid) ?? jid;
  }

  function toggle(jid: string) {
    if (multiple) selected = selected.includes(jid) ? selected.filter((j) => j !== jid) : [...selected, jid];
    else {
      selected = selected[0] === jid ? [] : [jid];
      open = false;
    }
    onchange?.(selected);
  }

  function openList() {
    if (!open) {
      open = true;
      void fetch(query);
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      openList();
      active = Math.min(results.length - 1, active + 1);
    } else if (e.key === "ArrowUp") active = Math.max(0, active - 1);
    else if (e.key === "Enter" && open) {
      const c = results[active];
      if (c) toggle(c.jid);
    } else if (e.key === "Escape" && open) {
      e.stopPropagation();
      open = false;
    } else return;
    e.preventDefault();
  }

  function onFocusOut(e: FocusEvent) {
    if (root && !root.contains(e.relatedTarget as Node | null)) open = false;
  }
</script>

<div class="relative" bind:this={root} onfocusout={onFocusOut} data-testid={testid}>
  <span class="label">{label}</span>
  <div class="input flex h-auto min-h-8 flex-wrap items-center gap-1 py-1">
    {#each selected as jid (jid)}
      <span class="badge badge-accent h-6 gap-1 pr-1">
        <span class="max-w-40 truncate">{nameOf(jid)}</span>
        <button type="button" class="rounded hover:bg-black/10" aria-label="Remove {nameOf(jid)}" onclick={() => toggle(jid)}><X size={12} /></button>
      </span>
    {/each}
    <input
      class="min-w-24 flex-1 bg-transparent text-[13px] outline-none"
      role="combobox"
      aria-expanded={open}
      aria-controls={listId}
      aria-label={label}
      placeholder={selected.length ? "" : placeholder}
      bind:value={query}
      onfocus={openList}
      oninput={() => {
        open = true;
        search(query);
      }}
      onkeydown={onKey}
    />
    <ChevronDown size={14} class="text-faint" />
  </div>
  {#if open}
    <ul id={listId} role="listbox" aria-multiselectable={multiple} class="absolute z-40 mt-1 max-h-64 w-full overflow-y-auto rounded-[var(--radius)] border border-border bg-surface p-1 shadow-[var(--shadow-lg)]">
      {#each results as c, i (c.jid)}
        {@const sel = selected.includes(c.jid)}
        <li role="option" aria-selected={sel}>
          <button
            type="button"
            tabindex="-1"
            class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] {i === active ? 'bg-surface-2' : ''}"
            onmouseenter={() => (active = i)}
            onmousedown={(e) => e.preventDefault()}
            onclick={() => toggle(c.jid)}
          >
            <Avatar name={c.name} id={c.jid} url={c.avatar_url} type={c.chat_type} size={22} />
            <span class="min-w-0 flex-1 truncate">{c.name}</span>
            {#if c.is_group}<span class="badge">group</span>{/if}
            {#if sel}<Check size={14} class="text-accent" />{/if}
          </button>
        </li>
      {:else}
        <li class="px-2 py-2 text-xs text-muted">No chats match</li>
      {/each}
    </ul>
  {/if}
</div>
