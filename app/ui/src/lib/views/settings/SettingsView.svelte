<script lang="ts">
  import { FolderPlus, X, Monitor, Sun, Moon } from "@lucide/svelte";
  import * as api from "../../api";
  import type { Settings } from "../../bindings/Settings";
  import Toggle from "../../components/Toggle.svelte";
  import Spinner from "../../components/Spinner.svelte";
  import Skeleton from "../../components/Skeleton.svelte";
  import { app } from "../../state/app.svelte";
  import { toasts } from "../../state/toasts.svelte";
  import { pickFolder } from "../../platform";

  let draft = $state<Settings | null>(null);
  let saving = $state(false);

  $effect(() => {
    if (app.settings && !draft) draft = $state.snapshot(app.settings) as Settings;
  });

  const dirty = $derived(!!draft && !!app.settings && JSON.stringify(draft) !== JSON.stringify(app.settings));
  const daysInvalid = $derived(!!draft && (!Number.isInteger(draft.backfill_days) || draft.backfill_days < 1 || draft.backfill_days > 3650));
  const rateInvalid = $derived(!!draft && (!Number.isInteger(draft.automation_max_per_hour) || draft.automation_max_per_hour < 0));

  async function save() {
    if (!draft || daysInvalid || rateInvalid) return;
    saving = true;
    try {
      const s = await api.setSettings($state.snapshot(draft) as Settings);
      app.settings = s;
      draft = $state.snapshot(s) as Settings;
      void app.setTheme(s.theme);
      toasts.success("Settings saved");
    } catch (e) {
      toasts.error(e, "Couldn't save settings");
    } finally {
      saving = false;
    }
  }

  function reset() {
    if (app.settings) draft = $state.snapshot(app.settings) as Settings;
  }

  async function addDir() {
    try {
      const p = await pickFolder("Add a folder to search for phone media");
      if (p && draft && !draft.link_extra_dirs.includes(p)) draft.link_extra_dirs = [...draft.link_extra_dirs, p];
    } catch (e) {
      toasts.error(e);
    }
  }

  const themes = [
    { id: "system", label: "System", icon: Monitor },
    { id: "light", label: "Light", icon: Sun },
    { id: "dark", label: "Dark", icon: Moon },
  ] as const;
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto max-w-3xl space-y-5 p-6">
    {#if !draft}
      <div class="card"><Skeleton rows={6} /></div>
    {:else}
      <form class="space-y-5" onsubmit={(e) => (e.preventDefault(), save())} data-testid="settings-form">
        <section class="card space-y-4 p-5" aria-labelledby="s-app">
          <h2 id="s-app" class="text-[15px] font-semibold">Appearance</h2>
          <div class="flex gap-2" role="radiogroup" aria-label="Theme">
            {#each themes as t (t.id)}
              <button
                type="button"
                role="radio"
                aria-checked={draft.theme === t.id}
                class="flex flex-1 flex-col items-center gap-1.5 rounded-[var(--radius)] border p-3 text-[13px] {draft.theme === t.id ? 'border-accent bg-accent-soft' : 'border-border hover:bg-surface-2'}"
                onclick={() => {
                  if (draft) draft.theme = t.id;
                  void app.setTheme(t.id);
                }}
                data-testid="theme-{t.id}"
              >
                <t.icon size={18} />{t.label}
              </button>
            {/each}
          </div>
        </section>

        <section class="card space-y-4 p-5" aria-labelledby="s-start">
          <h2 id="s-start" class="text-[15px] font-semibold">Startup & notifications</h2>
          <Toggle bind:checked={draft.autostart} label="Start wa-desk when I log in" />
          <Toggle bind:checked={draft.start_hidden} label="Start hidden in the tray" />
          <Toggle bind:checked={draft.desktop_notifications} label="Desktop notifications" description="Notify about new messages and automation activity." />
        </section>

        <section class="card space-y-4 p-5" aria-labelledby="s-send">
          <h2 id="s-send" class="text-[15px] font-semibold">Sending</h2>
          <Toggle bind:checked={draft.confirm_sends} label="Confirm before sending to groups or with attachments" />
          <div class="w-60">
            <label class="label" for="s-rate">Max automated sends per hour</label>
            <input id="s-rate" type="number" min="0" class="input" bind:value={draft.automation_max_per_hour} aria-invalid={rateInvalid ? "true" : undefined} />
          </div>
        </section>

        <section class="card space-y-4 p-5" aria-labelledby="s-media">
          <h2 id="s-media" class="text-[15px] font-semibold">Media</h2>
          <div class="w-60">
            <label class="label" for="s-days">Backfill window (days)</label>
            <input id="s-days" type="number" min="1" max="3650" class="input" bind:value={draft.backfill_days} aria-invalid={daysInvalid ? "true" : undefined} data-testid="settings-days" />
            {#if daysInvalid}<p class="mt-1 text-xs text-danger" role="alert">Enter 1–3650 days</p>{/if}
          </div>
          <div>
            <span class="label">Extra folders for phone media linking</span>
            <ul class="space-y-1">
              {#each draft.link_extra_dirs as d (d)}
                <li class="flex items-center gap-2 rounded-md bg-surface-2 px-2 py-1 text-[13px]">
                  <span class="min-w-0 flex-1 truncate font-mono text-xs">{d}</span>
                  <button type="button" class="icon-btn h-6 w-6" aria-label="Remove {d}" onclick={() => draft && (draft.link_extra_dirs = draft.link_extra_dirs.filter((x) => x !== d))}><X size={13} /></button>
                </li>
              {:else}
                <li class="text-xs text-muted">None — only the default WhatsApp folders are searched.</li>
              {/each}
            </ul>
            <button type="button" class="btn btn-sm mt-2" onclick={addDir} data-testid="add-dir"><FolderPlus size={13} /> Add folder…</button>
          </div>
        </section>

        <div class="sticky bottom-0 flex justify-end gap-2 border-t border-border bg-bg/90 py-3 backdrop-blur">
          {#if dirty}<span class="mr-auto self-center text-xs text-muted">Unsaved changes</span>{/if}
          <button type="button" class="btn" disabled={!dirty} onclick={reset}>Reset</button>
          <button type="submit" class="btn btn-primary" disabled={!dirty || saving || daysInvalid || rateInvalid} data-testid="save-settings">{#if saving}<Spinner size={13} />{/if} Save</button>
        </div>
      </form>
    {/if}
  </div>
</div>
