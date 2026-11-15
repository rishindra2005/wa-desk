<script lang="ts">
  import { RefreshCw, Smartphone, Wifi, WifiOff, QrCode } from "@lucide/svelte";
  import * as api from "../../api";
  import Spinner from "../../components/Spinner.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { timeAgo } from "../../utils/format";
  import { bridgeTone } from "./bridge";

  let busy = $state(false);
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });

  const conn = $derived(app.conn);
  const tone = $derived(bridgeTone(conn?.state));
  const qrLeft = $derived(conn?.qr_expires_at ? Math.max(0, conn.qr_expires_at - Math.floor(now / 1000)) : null);

  async function reconnect() {
    busy = true;
    try {
      app.conn = await api.reconnect();
    } catch (e) {
      toasts.error(e, "Reconnect failed");
    } finally {
      busy = false;
    }
  }
</script>

<section class="card p-5" aria-labelledby="bridge-h" data-testid="bridge-card">
  <div class="flex items-start gap-3">
    <div class="flex h-10 w-10 shrink-0 items-center justify-center rounded-full" style="background:color-mix(in srgb, {tone.color} 16%, transparent);color:{tone.color}">
      {#if conn?.connected}<Wifi size={19} />{:else if conn?.state === "pairing"}<QrCode size={19} />{:else}<WifiOff size={19} />{/if}
    </div>
    <div class="min-w-0 flex-1">
      <h2 id="bridge-h" class="flex items-center gap-2 text-[15px] font-semibold">
        WhatsApp bridge <span class="badge" style="background:color-mix(in srgb, {tone.color} 16%, transparent);color:{tone.color}" data-testid="bridge-state">{tone.label}</span>
      </h2>
      <p class="mt-0.5 text-[13px] text-muted">{conn?.message ?? "Checking…"}</p>
      {#if conn?.last_error}<p class="mt-1 font-mono text-[11px] text-danger">Last error: {conn.last_error}</p>{/if}
      {#if conn}<p class="mt-1 text-[11px] text-faint">Checked {timeAgo(conn.checked_at, now)}</p>{/if}
    </div>
    <button class="btn btn-sm" onclick={reconnect} disabled={busy} data-testid="reconnect">
      {#if busy}<Spinner size={12} />{:else}<RefreshCw size={13} />{/if} Reconnect
    </button>
  </div>
  {#if conn?.state === "pairing" && conn.qr_svg}
    <div class="mt-4 flex flex-wrap items-center gap-6 rounded-[var(--radius)] border border-border bg-surface-2/50 p-4" data-testid="pairing">
      <img src={conn.qr_svg} alt="Pairing QR code" class="h-56 w-56 rounded-md bg-white p-2" />
      <ol class="max-w-xs list-decimal space-y-1.5 pl-5 text-[13px]">
        <li>Open <strong>WhatsApp</strong> on your phone <Smartphone size={13} class="inline" /></li>
        <li>Go to <strong>Settings → Linked devices</strong></li>
        <li>Tap <strong>Link a device</strong> and scan this code</li>
        {#if qrLeft !== null}<li class="list-none text-xs text-muted">Code refreshes in {qrLeft}s</li>{/if}
      </ol>
    </div>
  {/if}
</section>
