<script lang="ts">
  import { CircleCheck, CircleAlert, Info, TriangleAlert, X } from "@lucide/svelte";
  import { toasts } from "../state/toasts.svelte";

  const icons = { success: CircleCheck, error: CircleAlert, info: Info, warn: TriangleAlert };
  const tones = { success: "text-accent", error: "text-danger", info: "text-info", warn: "text-warn" };
</script>

<div class="pointer-events-none fixed bottom-4 right-4 z-[70] flex w-[22rem] max-w-[calc(100vw-2rem)] flex-col gap-2" aria-live="polite" aria-relevant="additions">
  {#each toasts.items as t (t.id)}
    {@const Icon = icons[t.kind]}
    <div
      class="anim-in pointer-events-auto flex items-start gap-3 rounded-[var(--radius)] border border-border bg-surface p-3 shadow-[var(--shadow-lg)]"
      role={t.kind === "error" ? "alert" : "status"}
      data-testid="toast"
    >
      <span class="mt-0.5 {tones[t.kind]}"><Icon size={18} /></span>
      <div class="min-w-0 flex-1">
        <p class="text-[13px] font-semibold">{t.title}</p>
        {#if t.message}<p class="mt-0.5 break-words text-xs text-muted">{t.message}</p>{/if}
        {#if t.action}
          {@const action = t.action}
          <button
            class="mt-2 text-xs font-semibold text-accent-text hover:underline"
            onclick={() => {
              action.run();
              toasts.dismiss(t.id);
            }}>{action.label}</button
          >
        {/if}
      </div>
      <button class="icon-btn -mr-1 -mt-1 h-6 w-6" aria-label="Dismiss notification" onclick={() => toasts.dismiss(t.id)}><X size={14} /></button>
    </div>
  {/each}
</div>
