<!-- Profile picture with lazy loading, fade-in and an initials / type-icon fallback (failures are cached). -->
<script lang="ts">
  import { Users, Megaphone, Radio, Network, User } from "@lucide/svelte";
  import type { ChatType } from "../bindings/ChatType";
  import { avatarUrl, avatarFailed, avatarLoaded, markAvatarFailed, markAvatarLoaded } from "../avatar";
  import { colorHue } from "../utils/misc";
  import { initials } from "../utils/text";

  interface Props {
    name: string;
    id?: string;
    size?: number;
    /** @deprecated use `type` */
    group?: boolean;
    type?: ChatType;
    /** Explicit URL; `null` forces the fallback; omitted = derive from `id`. */
    url?: string | null;
    onclick?: (e: MouseEvent) => void;
    label?: string;
  }
  let { name, id, size = 40, group = false, type, url, onclick, label }: Props = $props();

  const kind = $derived<ChatType>(type ?? (group ? "group" : "direct"));
  const hue = $derived(colorHue(id ?? name));
  const text = $derived(initials(name));
  const src = $derived(url === null ? null : id || url ? avatarUrl(id ?? "", url) : null);
  let failedTick = $state(0);
  const failed = $derived.by(() => {
    void failedTick;
    return !src || avatarFailed(src);
  });
  let ready = $state(false);
  $effect(() => {
    ready = !!src && avatarLoaded(src);
  });
  const iconSize = $derived(Math.round(size * 0.5));
</script>

{#snippet inner()}
  <span
    class="relative flex shrink-0 select-none items-center justify-center overflow-hidden rounded-full font-semibold text-white"
    style="width:{size}px;height:{size}px;font-size:{Math.round(size * 0.38)}px;background:hsl({hue} var(--avatar-s) var(--avatar-l))"
    aria-hidden={onclick ? undefined : "true"}
    data-testid="avatar"
    data-state={failed ? "fallback" : ready ? "image" : "loading"}
  >
    {#if failed}
      {#if kind === "group"}<Users size={iconSize} data-testid="avatar-icon" />
      {:else if kind === "community"}<Network size={iconSize} data-testid="avatar-icon" />
      {:else if kind === "channel"}<Megaphone size={iconSize} data-testid="avatar-icon" />
      {:else if kind === "broadcast"}<Radio size={iconSize} data-testid="avatar-icon" />
      {:else if text === "#"}<User size={iconSize} data-testid="avatar-icon" />
      {:else}<span data-testid="avatar-initials">{text}</span>{/if}
    {/if}
    {#if src && !failed}
      <img
        {src}
        alt=""
        loading="lazy"
        decoding="async"
        draggable="false"
        class="absolute inset-0 h-full w-full object-cover transition-opacity duration-200 {ready ? 'opacity-100' : 'opacity-0'}"
        onload={(e) => {
          markAvatarLoaded(e.currentTarget.getAttribute("src") ?? "");
          ready = true;
        }}
        onerror={(e) => {
          markAvatarFailed(e.currentTarget.getAttribute("src") ?? "");
          failedTick++;
        }}
      />
    {/if}
  </span>
{/snippet}

{#if onclick}
  <button type="button" class="shrink-0 rounded-full" aria-label={label ?? `${name} profile`} {onclick} data-testid="avatar-btn">{@render inner()}</button>
{:else}
  {@render inner()}
{/if}
