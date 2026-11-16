<!-- WhatsApp-style delivery ticks: ✓ sent, ✓✓ delivered, blue ✓✓ read/played, clock while sending, red ! on failure. -->
<script lang="ts" module>
  import type { MessageStatus } from "../bindings/MessageStatus";
  export type TickState = MessageStatus | "pending" | "failed";

  export const TICK_TEXT: Record<TickState, string> = {
    pending: "Sending…",
    failed: "Not sent",
    sent: "Sent",
    delivered: "Delivered",
    read: "Read",
    played: "Played",
  };

  /** Colour class name per state (grey until read). */
  export function tickTone(s: TickState): "grey" | "blue" | "red" {
    return s === "read" || s === "played" ? "blue" : s === "failed" ? "red" : "grey";
  }
</script>

<script lang="ts">
  import { Check, CheckCheck, Clock, CircleAlert } from "@lucide/svelte";

  let { status, size = 15 }: { status: TickState; size?: number } = $props();
  const tone = $derived(tickTone(status));
  const text = $derived(TICK_TEXT[status]);
</script>

<span
  class="inline-flex shrink-0 items-center align-middle {tone === 'blue' ? 'text-[var(--tick-read)]' : tone === 'red' ? 'text-danger' : 'text-faint'}"
  role="img"
  aria-label={text}
  title={text}
  data-testid="tick"
  data-status={status}
  data-tone={tone}
>
  {#if status === "pending"}<Clock size={size - 2} />
  {:else if status === "failed"}<CircleAlert size={size} />
  {:else if status === "sent"}<Check size={size} />
  {:else}<CheckCheck size={size} />{/if}
</span>
