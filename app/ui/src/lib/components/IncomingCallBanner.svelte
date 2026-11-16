<!-- Slide-in banners for calls ringing on the phone. They can only be declined here, never answered. -->
<script lang="ts">
  import { Phone, Video, PhoneOff, X } from "@lucide/svelte";
  import Avatar from "./Avatar.svelte";
  import Spinner from "./Spinner.svelte";
  import { calls } from "../state/calls.svelte";
  import { ringLabel } from "../utils/calls";
  import { chatLabel, displayName } from "../utils/format";
</script>

{#if calls.ringing.length}
  <div class="shrink-0 border-b border-border bg-surface" role="region" aria-label="Incoming calls" data-testid="call-banners">
    {#each calls.ringing as r (r.call.call_id)}
      {@const c = r.call}
      {@const name = c.is_group ? chatLabel({ name: c.chat_name, jid: c.chat_jid, chat_type: "group" }) : displayName(c.caller_name, c.caller_jid)}
      <div class="call-banner flex items-center gap-3 bg-accent-soft px-4 py-2.5" role="alert" data-testid="call-banner" data-call-id={c.call_id} data-media={c.media}>
        <span class="relative flex shrink-0 items-center justify-center">
          <Avatar name={name} id={c.chat_jid} url={c.avatar_url} type={c.is_group ? "group" : "direct"} size={40} />
          <span class="ring-icon absolute -bottom-1 -right-1 flex h-5 w-5 items-center justify-center rounded-full bg-accent text-accent-fg" aria-hidden="true">
            {#if c.media === "video"}<Video size={11} />{:else}<Phone size={11} />{/if}
          </span>
        </span>
        <div class="min-w-0 flex-1">
          <p class="truncate text-[14px] font-semibold" data-testid="call-banner-name" dir="auto">{name}</p>
          <p class="truncate text-[12.5px] text-muted">
            <span class="font-medium text-accent-text" data-testid="call-banner-label">
              {ringLabel(c)}{c.is_group ? ` · ${displayName(c.caller_name, c.caller_jid)} is calling` : " · ringing"}
            </span>
            · <span data-testid="call-banner-hint">Answer on your phone</span>
          </p>
          {#if r.error}<p class="truncate text-[12px] text-danger" data-testid="call-banner-error">{r.error}</p>{/if}
        </div>
        <button
          class="btn btn-danger btn-sm"
          disabled={r.busy || !!r.error}
          title={r.error ?? "Decline this call (the caller sees it as declined)"}
          aria-label="Decline call"
          onclick={() => void calls.decline(c.call_id)}
          data-testid="decline-call"
        >
          {#if r.busy}<Spinner size={13} label="Declining" />{:else}<PhoneOff size={13} />{/if} Decline
        </button>
        <button class="icon-btn" aria-label="Dismiss" title="Dismiss (the call keeps ringing on your phone)" onclick={() => calls.dismiss(c.call_id)} data-testid="dismiss-call"><X size={16} /></button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .call-banner {
    animation: banner-in 0.28s cubic-bezier(0.2, 0.8, 0.2, 1);
  }
  .ring-icon {
    animation: ring-shake 1.2s ease-in-out infinite;
    transform-origin: 50% 50%;
  }
  @keyframes banner-in {
    from {
      opacity: 0;
      transform: translateY(-100%);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  @keyframes ring-shake {
    0%,
    60%,
    100% {
      transform: rotate(0);
    }
    10%,
    30% {
      transform: rotate(-16deg);
    }
    20%,
    40% {
      transform: rotate(16deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .call-banner,
    .ring-icon {
      animation: none;
    }
  }
</style>
