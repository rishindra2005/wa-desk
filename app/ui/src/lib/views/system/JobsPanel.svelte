<script lang="ts">
  import { Download, Smartphone, Link2, DatabaseZap, ImagePlus, Play } from "@lucide/svelte";
  import * as api from "../../api";
  import type { JobSpec } from "../../bindings/JobSpec";
  import type { MediaKind } from "../../bindings/MediaKind";
  import JobCard from "../../components/JobCard.svelte";
  import Modal from "../../components/Modal.svelte";
  import EmptyState from "../../components/EmptyState.svelte";
  import Toggle from "../../components/Toggle.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { mediaEmoji } from "../../utils/format";

  let form = $state<"backfill" | "phone_sync" | null>(null);
  let days = $state(app.settings?.backfill_days ?? 30);
  let kinds = $state<MediaKind[]>(["image", "video", "audio", "document"]);
  let workers = $state(4);
  let retryFailed = $state(false);
  let starting = $state(false);

  const running = $derived(app.runningJobs);
  const history = $derived(app.jobs.filter((j) => j.status !== "running" && j.status !== "queued"));

  async function start(spec: JobSpec) {
    starting = true;
    try {
      const job = await api.startJob(spec);
      app.upsertJob(job);
      form = null;
      toasts.info(`Started: ${job.label}`);
    } catch (e) {
      toasts.error(e, "Couldn't start job");
    } finally {
      starting = false;
    }
  }

  function toggleKind(k: MediaKind) {
    kinds = kinds.includes(k) ? kinds.filter((x) => x !== k) : [...kinds, k];
  }

  const actions = [
    { id: "backfill", label: "Backfill media", desc: "Download missing media from WhatsApp", icon: Download, run: () => (form = "backfill") },
    { id: "phone_sync", label: "Phone sync (USB)", desc: "Copy media from your phone over adb", icon: Smartphone, run: () => (form = "phone_sync") },
    { id: "link", label: "Link phone media", desc: "Match files in your folders to messages", icon: Link2, run: () => start({ type: "link_phone_media", extra_dirs: app.settings?.link_extra_dirs ?? [] }) },
    { id: "reindex", label: "Rebuild index", desc: "Rebuild the full-text search index", icon: DatabaseZap, run: () => start({ type: "reindex" }) },
    { id: "thumbs", label: "Thumbnails", desc: "Pre-generate image/video/PDF thumbnails", icon: ImagePlus, run: () => start({ type: "thumbnails" }) },
  ] as const;
</script>

<div class="space-y-5">
  <div class="grid gap-2 sm:grid-cols-2 xl:grid-cols-5">
    {#each actions as a (a.id)}
      <button class="card flex flex-col items-start gap-1 p-3 text-left transition-colors hover:border-border-strong hover:bg-surface-2" onclick={a.run} data-testid="job-action-{a.id}">
        <a.icon size={18} class="text-accent" />
        <span class="text-[13px] font-medium">{a.label}</span>
        <span class="text-[11px] text-muted">{a.desc}</span>
      </button>
    {/each}
  </div>

  <section aria-labelledby="running-h" class="space-y-2">
    <h3 id="running-h" class="section-title">Running</h3>
    {#each running as j (j.id)}
      <JobCard job={j} />
    {:else}
      <p class="text-xs text-muted">No jobs running.</p>
    {/each}
  </section>

  <section aria-labelledby="hist-h" class="space-y-2">
    <h3 id="hist-h" class="section-title">History</h3>
    {#each history as j (j.id)}
      <JobCard job={j} />
    {:else}
      <div class="card"><EmptyState title="No finished jobs" /></div>
    {/each}
  </section>
</div>

{#if form === "backfill"}
  <Modal title="Backfill media" onclose={() => (form = null)}>
    <div class="space-y-4">
      <div class="grid grid-cols-2 gap-3">
        <label class="text-xs">Days back<input type="number" min="1" max="3650" class="input mt-1" bind:value={days} data-testid="backfill-days" /></label>
        <label class="text-xs">Parallel downloads<input type="number" min="1" max="16" class="input mt-1" bind:value={workers} /></label>
      </div>
      <div class="flex flex-wrap gap-1.5" role="group" aria-label="Media kinds">
        {#each ["image", "video", "audio", "document", "sticker"] as const as k (k)}
          <button class="chip capitalize" aria-pressed={kinds.includes(k)} onclick={() => toggleKind(k)}>{mediaEmoji(k)} {k}</button>
        {/each}
      </div>
      <Toggle bind:checked={retryFailed} label="Retry previously failed downloads" />
    </div>
    {#snippet footer()}
      <button class="btn" onclick={() => (form = null)}>Cancel</button>
      <button class="btn btn-primary" disabled={starting || kinds.length === 0 || days < 1} onclick={() => start({ type: "backfill", days, kinds: $state.snapshot(kinds), workers, retry_failed: retryFailed })} data-testid="start-backfill"><Play size={14} /> Start</button>
    {/snippet}
  </Modal>
{:else if form === "phone_sync"}
  <Modal title="Phone sync over USB" onclose={() => (form = null)}>
    <ol class="list-decimal space-y-1.5 pl-5 text-[13px]">
      <li>Enable <strong>Developer options → USB debugging</strong> on your Android phone.</li>
      <li>Connect it with a USB cable and unlock the screen.</li>
      <li>Accept the <strong>“Allow USB debugging?”</strong> prompt on the phone.</li>
      <li>Start the sync — WhatsApp media folders are copied and linked to messages.</li>
    </ol>
    {#snippet footer()}
      <button class="btn" onclick={() => (form = null)}>Cancel</button>
      <button class="btn btn-primary" disabled={starting} onclick={() => start({ type: "phone_sync" })}><Play size={14} /> Start sync</button>
    {/snippet}
  </Modal>
{/if}
