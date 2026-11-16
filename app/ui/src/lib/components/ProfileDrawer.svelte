<!-- Right-side info drawer: contact / group / community / channel details, with chat-scoped media, docs, links and starred. -->
<script lang="ts">
  import { untrack } from "svelte";
  import {
    X, ArrowLeft, Copy, Search, FileDown, MessageSquare, BadgeCheck, Lock, Megaphone, Users, Link as LinkIcon,
    Star, FileText, Images, Mail, Globe, MapPin, Briefcase, ChevronRight, Network, Phone, PhoneIncoming, PhoneMissed, PhoneOff, PhoneOutgoing, Video,
  } from "@lucide/svelte";
  import * as api from "../api";
  import type { CallRecord } from "../bindings/CallRecord";
  import type { ChatStats } from "../bindings/ChatStats";
  import type { ChatSummary } from "../bindings/ChatSummary";
  import type { LinkItem } from "../bindings/LinkItem";
  import type { MediaItem } from "../bindings/MediaItem";
  import type { Profile } from "../bindings/Profile";
  import type { SearchHit } from "../bindings/SearchHit";
  import Avatar from "./Avatar.svelte";
  import Lightbox from "./Lightbox.svelte";
  import Skeleton from "./Skeleton.svelte";
  import Spinner from "./Spinner.svelte";
  import DocIcon from "./DocIcon.svelte";
  import { avatarUrl, fullAvatarUrl, avatarFailed } from "../avatar";
  import { openLink } from "../platform";
  import { app } from "../state/app.svelte";
  import { toasts } from "../state/toasts.svelte";
  import { chatLabel, dateTime, displayName, formatCount, formatPhone, formatSize, shortDate } from "../utils/format";
  import { copyText } from "../utils/misc";
  import { CALL_FROM_PHONE_HINT, callDetail, callTitle, direction } from "../utils/calls";

  type Tab = "media" | "docs" | "links" | "starred" | "calls";
  const PAGE = 60;

  let profile = $state<Profile | null>(null);
  let fallback = $state<ChatSummary | null>(null);
  let loading = $state(true);
  let stats = $state<ChatStats | null>(null);
  let tab = $state<Tab>("media");
  let query = $state("");
  let shown = $state(60);
  let lightbox = $state<string | null>(null);
  let seq = 0;

  let media = $state<MediaItem[]>([]);
  let docs = $state<MediaItem[]>([]);
  let links = $state<LinkItem[]>([]);
  let starred = $state<SearchHit[]>([]);
  let callList = $state<CallRecord[]>([]);
  let cursors = $state<Record<Tab, string | null>>({ media: null, docs: null, links: null, starred: null, calls: null });
  let tabLoading = $state(false);

  const jid = $derived(app.profileJid);

  async function load(j: string) {
    const my = ++seq;
    loading = true;
    profile = null;
    fallback = null;
    stats = null;
    query = "";
    shown = 60;
    media = docs = [];
    links = [];
    starred = [];
    callList = [];
    cursors = { media: null, docs: null, links: null, starred: null, calls: null };
    void api.chatStats(j).then((s) => my === seq && (stats = s), () => {});
    api.getChat(j).then((c) => my === seq && (fallback = c), () => {});
    try {
      const p = await api.getProfile(j);
      if (my === seq) profile = p;
    } catch {
      /* local fallback below */
    } finally {
      if (my === seq) loading = false;
    }
    if (my === seq) void loadTab(true);
  }

  $effect(() => {
    const j = jid;
    if (j) untrack(() => void load(j));
  });

  async function loadTab(reset = false) {
    const j = jid;
    if (!j) return;
    const my = seq;
    const t = tab;
    if (!reset && cursors[t] === null && listFor(t).length > 0) return;
    tabLoading = true;
    try {
      const cursor = reset ? null : cursors[t];
      if (t === "media" || t === "docs") {
        const p = await api.listMedia({
          chat_jid: j, kinds: t === "media" ? ["image", "video"] : ["document"], text: null, after: null, before: null,
          available: null, cursor, limit: PAGE,
        });
        if (my !== seq) return;
        if (t === "media") media = reset ? p.items : [...media, ...p.items];
        else docs = reset ? p.items : [...docs, ...p.items];
        cursors[t] = p.next_cursor;
      } else if (t === "calls") {
        const p = await api.listCalls({ chat_jid: j, missed_only: false, media: null, cursor, limit: PAGE });
        if (my !== seq) return;
        callList = reset ? p.items : [...callList, ...p.items];
        cursors[t] = p.next_cursor;
      } else if (t === "links") {
        const p = await api.listLinks(j, cursor, PAGE);
        if (my !== seq) return;
        links = reset ? p.items : [...links, ...p.items];
        cursors[t] = p.next_cursor;
      } else {
        const p = await api.listStarred(j, cursor, PAGE);
        if (my !== seq) return;
        starred = reset ? p.items : [...starred, ...p.items];
        cursors[t] = p.next_cursor;
      }
    } catch (e) {
      toasts.error(e);
    } finally {
      if (my === seq) tabLoading = false;
    }
  }

  function listFor(t: Tab): unknown[] {
    return t === "media" ? media : t === "docs" ? docs : t === "links" ? links : t === "calls" ? callList : starred;
  }

  function pickTab(t: Tab) {
    tab = t;
    void loadTab(true);
  }

  // Normalised view-model so one template serves every profile type.
  const kind = $derived(profile ? (profile.type === "group" ? profile.chat_type : profile.type) : (fallback?.chat_type ?? "direct"));
  const name = $derived(
    profile ? chatLabel({ name: profile.name, jid: profile.jid, chat_type: kind }) : fallback ? chatLabel(fallback) : jid ? formatPhone(jid) : "",
  );
  const avatar = $derived(profile?.avatar_url ?? fallback?.avatar_url ?? (jid ? avatarUrl(jid) : ""));
  const heading = $derived(
    { direct: "Contact info", contact: "Contact info", group: "Group info", community: "Community info", channel: "Channel info", broadcast: "Broadcast list info", status: "Info" }[kind] ?? "Info",
  );
  const group = $derived(profile?.type === "group" ? profile : null);
  const channel = $derived(profile?.type === "channel" ? profile : null);
  const contact = $derived(profile?.type === "contact" ? profile : null);

  const members = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const list = group?.participants ?? [];
    if (!q) return list;
    return list.filter((p) => `${p.name} ${p.phone ?? ""} ${p.jid}`.toLowerCase().includes(q));
  });

  async function copy(text: string, what: string) {
    if (await copyText(text)) toasts.success(`${what} copied`);
  }
  function openChat() {
    if (jid) app.openChat(jid);
  }
  const canFull = $derived(!!avatar && !avatarFailed(avatar));
</script>

<aside class="flex h-full w-[380px] shrink-0 flex-col border-l border-border bg-surface anim-in" aria-label={heading} data-testid="profile-drawer">
  <header class="flex h-14 shrink-0 items-center gap-1 border-b border-border px-2">
    {#if app.profileBack.length}
      <button class="icon-btn" aria-label="Back" onclick={() => app.profileGoBack()} data-testid="profile-back"><ArrowLeft size={18} /></button>
    {/if}
    <h2 class="flex-1 px-2 text-[15px] font-semibold" data-testid="profile-heading">{heading}</h2>
    <button class="icon-btn" aria-label="Close info" onclick={() => app.closeProfile()} data-testid="profile-close"><X size={18} /></button>
  </header>

  <div class="min-h-0 flex-1 overflow-y-auto" data-testid="profile-body">
    {#if loading && !fallback}
      <div class="flex flex-col items-center gap-3 p-6" data-testid="profile-skeleton">
        <div class="skeleton h-28 w-28 rounded-full"></div>
        <div class="skeleton h-5 w-40"></div>
        <div class="skeleton h-3.5 w-28"></div>
      </div>
      <Skeleton rows={4} />
    {:else if jid}
      <section class="flex flex-col items-center gap-1 px-6 pb-4 pt-6 text-center">
        <Avatar
          name={name}
          id={jid}
          url={avatar}
          type={kind === "contact" || kind === "status" ? "direct" : kind}
          size={120}
          onclick={canFull ? () => (lightbox = fullAvatarUrl(avatar)) : undefined}
          label="View profile picture"
        />
        <h3 class="mt-2 flex items-center gap-1.5 text-[19px] font-semibold leading-tight" data-testid="profile-name" dir="auto">
          {name}
          {#if channel?.verified}<BadgeCheck size={18} class="text-info" aria-label="Verified" data-testid="verified-badge" />{/if}
        </h3>
        {#if contact}
          {#if contact.phone}
            <p class="flex items-center gap-1 text-[14px] text-muted" data-testid="profile-phone">
              {contact.phone}
              <button class="icon-btn h-6 w-6" aria-label="Copy phone number" onclick={() => contact.phone && copy(contact.phone, "Number")} data-testid="copy-phone"><Copy size={13} /></button>
            </p>
          {/if}
          {#if contact.push_name && contact.push_name !== contact.name}<p class="text-xs text-faint">~{contact.push_name}</p>{/if}
          {#if contact.is_me}<span class="badge badge-accent">You</span>{/if}
        {:else if group}
          <p class="text-[13px] text-muted" data-testid="profile-subtitle">
            {group.chat_type === "community" ? "Community" : group.chat_type === "broadcast" ? "Broadcast list" : "Group"} · {formatCount(group.participant_count)} participants
          </p>
        {:else if channel}
          <p class="text-[13px] text-muted" data-testid="profile-subtitle">Channel · {channel.subscribers != null ? `${formatCount(channel.subscribers)} subscribers` : "Subscribers hidden"}</p>
        {:else if fallback}
          <p class="text-xs text-faint">Showing local data</p>
        {/if}
        <div class="mt-3 flex flex-wrap justify-center gap-2">
          {#if app.openChatJid !== jid}<button class="btn btn-sm" onclick={openChat} data-testid="profile-open-chat"><MessageSquare size={13} /> Open chat</button>{/if}
          <button class="btn btn-sm" onclick={() => jid && app.searchInChat(jid)} data-testid="profile-search"><Search size={13} /> Search</button>
          <button class="btn btn-sm" onclick={() => jid && app.openExport(jid, name)} data-testid="profile-export"><FileDown size={13} /> Export chat</button>
        </div>
      </section>

      {#if contact?.about}
        <section class="border-t border-border px-5 py-3">
          <p class="section-title">About</p>
          <p class="mt-1 text-[14px]" dir="auto" data-testid="profile-about">{contact.about}</p>
        </section>
      {/if}

      {#if contact?.business}
        {@const b = contact.business}
        <section class="space-y-1.5 border-t border-border px-5 py-3" data-testid="profile-business">
          <p class="section-title flex items-center gap-1.5"><Briefcase size={12} /> Business</p>
          {#if b.category}<p class="text-[13px] font-medium">{b.category}</p>{/if}
          {#if b.description}<p class="text-[13px] text-muted">{b.description}</p>{/if}
          {#if b.email}<p class="flex items-center gap-2 text-[13px]"><Mail size={13} class="text-faint" /> {b.email}</p>{/if}
          {#if b.website}
            <button class="flex items-center gap-2 text-[13px] text-info hover:underline" onclick={() => b.website && openLink(b.website).catch((e) => toasts.error(e))}><Globe size={13} class="text-faint" /> {b.website}</button>
          {/if}
          {#if b.address}<p class="flex items-start gap-2 text-[13px]"><MapPin size={13} class="mt-0.5 shrink-0 text-faint" /> {b.address}</p>{/if}
        </section>
      {/if}

      {#if group || channel}
        {@const desc = group?.description ?? channel?.description}
        <section class="space-y-1.5 border-t border-border px-5 py-3">
          <p class="section-title">Description</p>
          <p class="whitespace-pre-wrap text-[13.5px] {desc ? '' : 'text-faint'}" dir="auto" data-testid="profile-description">{desc ?? "No description"}</p>
          {#if group}
            <div class="flex flex-wrap gap-1.5 pt-1">
              {#if group.announce_only}<span class="badge badge-warn" data-testid="badge-announce"><Megaphone size={11} class="mr-1" /> Only admins can send</span>{/if}
              {#if group.locked}<span class="badge" data-testid="badge-locked"><Lock size={11} class="mr-1" /> Only admins can edit info</span>{/if}
            </div>
            {#if group.created_at}
              <p class="text-xs text-faint" data-testid="profile-created">
                Created {shortDate(group.created_at, Date.now(), true)}{group.created_by ? ` by ${formatPhone(group.created_by)}` : ""}
              </p>
            {/if}
          {/if}
        </section>
      {/if}

      {#if group?.parent}
        <section class="border-t border-border px-5 py-3">
          <p class="section-title">Part of community</p>
          <button class="mt-1.5 flex w-full items-center gap-2.5 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => group?.parent && app.openProfile(group.parent.jid, true)} data-testid="profile-parent">
            <Avatar name={group.parent.name} id={group.parent.jid} type="community" size={32} />
            <span class="min-w-0 flex-1 truncate text-[13.5px] font-medium">{group.parent.name}</span>
            <ChevronRight size={15} class="text-faint" />
          </button>
        </section>
      {/if}

      {#if group && group.sub_groups.length}
        <section class="border-t border-border px-5 py-3" data-testid="profile-subgroups">
          <p class="section-title">Groups in this community · {group.sub_groups.length}</p>
          <ul class="mt-1.5">
            {#each group.sub_groups as g (g.jid)}
              <li>
                <button class="flex w-full items-center gap-2.5 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => app.openChat(g.jid)} data-testid="subgroup">
                  <Avatar name={g.name} id={g.jid} type="group" size={32} />
                  <span class="min-w-0 flex-1 truncate text-[13.5px]">{g.name}</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      <section class="border-t border-border px-3 py-2" aria-label="Chat content">
        <div class="tabbar w-full" role="tablist" aria-label="Shared content">
          {#each [["media", "Media", stats?.media], ["docs", "Docs", stats?.docs], ["links", "Links", stats?.links], ["starred", "Starred", stats?.starred], ["calls", "Calls", undefined]] as const as [id, label, n] (id)}
            <button class="flex-1" role="tab" aria-selected={tab === id} onclick={() => pickTab(id)} data-testid="ptab-{id}">
              {label}{#if n !== undefined} <span class="text-faint" data-testid="count-{id}">{n}</span>{/if}
            </button>
          {/each}
        </div>
        <div class="mt-2 min-h-16" data-testid="ptab-panel">
          {#if tab === "media"}
            {#if media.length === 0 && !tabLoading}<p class="py-4 text-center text-xs text-faint">No media</p>{/if}
            <div class="grid grid-cols-3 gap-1">
              {#each media as it (it.message_id)}
                <button class="relative aspect-square overflow-hidden rounded bg-surface-3" onclick={() => app.jumpTo(it.chat_jid, it.message_id)} aria-label="Go to {it.media.kind}" data-testid="pmedia">
                  {#if it.media.thumb_url ?? it.media.url}<img src={it.media.thumb_url ?? it.media.url} alt="" loading="lazy" class="h-full w-full object-cover" />{/if}
                  {#if it.media.kind === "video"}<span class="absolute bottom-1 left-1 rounded bg-black/60 px-1 text-[10px] text-white">Video</span>{/if}
                </button>
              {/each}
            </div>
          {:else if tab === "docs"}
            {#if docs.length === 0 && !tabLoading}<p class="py-4 text-center text-xs text-faint">No documents</p>{/if}
            <ul>
              {#each docs as it (it.message_id)}
                <li>
                  <button class="flex w-full items-center gap-2.5 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => app.jumpTo(it.chat_jid, it.message_id)} data-testid="pdoc">
                    <DocIcon filename={it.media.filename} mime={it.media.mime} />
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-[13px] font-medium">{it.media.filename ?? "Document"}</span>
                      <span class="flex items-center gap-1 text-[11px] text-muted">
                        {#if it.sender_jid}<Avatar name={it.sender_name} id={it.sender_jid} size={14} />{/if}
                        <span class="truncate">{it.is_from_me ? "You" : displayName(it.sender_name, it.sender_jid)} · {formatSize(it.media.size)} · {shortDate(it.ts)}</span>
                      </span>
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else if tab === "links"}
            {#if links.length === 0 && !tabLoading}<p class="py-4 text-center text-xs text-faint">No links</p>{/if}
            <ul>
              {#each links as l (l.message_id + l.url)}
                <li>
                  <button class="flex w-full items-start gap-2.5 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => openLink(l.url.startsWith("http") ? l.url : `https://${l.url}`).catch((e) => toasts.error(e))} data-testid="plink">
                    <LinkIcon size={15} class="mt-0.5 shrink-0 text-info" />
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-[13px] text-info">{l.url}</span>
                      <span class="block truncate text-[11px] text-muted">{l.sender_name} · {shortDate(l.ts)}</span>
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {:else if tab === "calls"}
            {#if callList.length === 0 && !tabLoading}<p class="py-4 text-center text-xs text-faint" data-testid="pcalls-empty">No calls</p>{/if}
            <ul>
              {#each callList as c (c.call_id + c.chat_jid)}
                {@const dir = direction(c)}
                {@const bad = dir === "missed" || dir === "declined"}
                <li class="flex items-center gap-2.5 rounded-md p-1.5" data-testid="pcall" data-result={c.result}>
                  <span class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-surface-2 {bad ? 'text-danger' : 'text-accent-text'}">
                    {#if dir === "missed"}<PhoneMissed size={15} />{:else if dir === "declined"}<PhoneOff size={15} />{:else if dir === "outgoing"}<PhoneOutgoing size={15} />{:else}<PhoneIncoming size={15} />{/if}
                  </span>
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[13px] font-medium {bad ? 'text-danger' : ''}">{callTitle(c)}</span>
                    <span class="block truncate text-[11.5px] text-muted">{callDetail(c)} · {dateTime(c.ts)}</span>
                  </span>
                  {#if c.media === "video"}<Video size={14} class="shrink-0 text-faint" />{:else}<Phone size={14} class="shrink-0 text-faint" />{/if}
                </li>
              {/each}
            </ul>
            <p class="mt-2 px-1.5 text-center text-[11px] text-faint" title={CALL_FROM_PHONE_HINT}>{CALL_FROM_PHONE_HINT}</p>
          {:else}
            {#if starred.length === 0 && !tabLoading}<p class="py-4 text-center text-xs text-faint">No starred messages</p>{/if}
            <ul>
              {#each starred as h (h.message.id)}
                <li>
                  <button class="flex w-full items-start gap-2 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => app.jumpTo(h.message.chat_jid, h.message.id)} data-testid="pstar">
                    <Star size={14} class="mt-0.5 shrink-0 fill-warn text-warn" />
                    <span class="min-w-0 flex-1">
                      <span class="block truncate text-[12px] text-muted">{h.message.is_from_me ? "You" : displayName(h.message.sender_name, h.message.sender_jid)} · {dateTime(h.message.ts)}</span>
                      <span class="line-clamp-2 text-[13px]">{h.message.content || h.message.media?.filename || "Media"}</span>
                    </span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
          {#if tabLoading}<div class="flex justify-center py-3"><Spinner size={16} /></div>{/if}
          {#if cursors[tab] && !tabLoading}
            <button class="btn btn-sm mt-2 w-full" onclick={() => loadTab(false)}>Load more</button>
          {/if}
        </div>
      </section>

      {#if contact && contact.common_groups.length}
        <section class="border-t border-border px-5 py-3" data-testid="common-groups">
          <p class="section-title">{contact.common_groups.length} group{contact.common_groups.length === 1 ? "" : "s"} in common</p>
          <ul class="mt-1.5">
            {#each contact.common_groups as g (g.jid)}
              <li>
                <button class="flex w-full items-center gap-2.5 rounded-md p-1.5 text-left hover:bg-surface-2" onclick={() => app.openChat(g.jid)} data-testid="common-group">
                  <Avatar name={g.name} id={g.jid} type={g.chat_type} size={32} />
                  <span class="min-w-0 flex-1 truncate text-[13.5px]">{g.name}</span>
                </button>
              </li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if group}
        <section class="border-t border-border px-3 py-3" data-testid="participants">
          <p class="section-title flex items-center gap-1.5 px-2"><Users size={12} /> {formatCount(group.participant_count)} participants</p>
          <div class="relative mt-2 px-2">
            <Search size={14} class="pointer-events-none absolute left-4.5 top-1/2 -translate-y-1/2 text-faint" />
            <input class="input pl-8" placeholder="Search participants" aria-label="Search participants" bind:value={query} oninput={() => (shown = 60)} data-testid="participant-search" />
          </div>
          <ul class="mt-1.5">
            {#each members.slice(0, shown) as p (p.jid)}
              {@const label = p.name || displayName("", p.jid)}
              <li>
                <button
                  class="flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left hover:bg-surface-2"
                  onclick={() => !p.is_me && app.openProfile(p.jid, true)}
                  data-testid="participant"
                  data-jid={p.jid}
                >
                  <Avatar name={label} id={p.jid} url={p.avatar_url || undefined} size={34} />
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-[13.5px] font-medium" dir="auto">{p.is_me ? "You" : label}</span>
                    <span class="block truncate text-[11.5px] text-muted">{p.phone ?? (p.jid.endsWith("@lid") ? "Phone hidden" : "")}</span>
                  </span>
                  {#if p.is_super_admin}<span class="badge badge-accent" data-testid="admin-badge">Group admin</span>{:else if p.is_admin}<span class="badge badge-accent" data-testid="admin-badge">Admin</span>{/if}
                </button>
              </li>
            {/each}
          </ul>
          {#if members.length === 0}<p class="py-3 text-center text-xs text-faint">No participants match</p>{/if}
          {#if members.length > shown}
            <button class="btn btn-sm mx-2 mt-1 w-[calc(100%-1rem)]" onclick={() => (shown += 100)}>Show {Math.min(100, members.length - shown)} more</button>
          {/if}
        </section>
      {/if}
    {/if}
  </div>
</aside>

{#if lightbox}
  <Lightbox items={[{ key: "avatar", url: lightbox, kind: "image", title: name }]} index={0} onclose={() => (lightbox = null)} />
{/if}
