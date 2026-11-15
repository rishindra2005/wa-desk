<!-- Free-text chips (keywords, senders). Enter or comma adds; Backspace removes the last. -->
<script lang="ts">
  import { X } from "@lucide/svelte";

  interface Props {
    values: string[];
    label: string;
    placeholder?: string;
    validate?: (v: string) => string | null;
    onchange?: (v: string[]) => void;
    testid?: string;
  }
  let { values = $bindable(), label, placeholder = "Type and press Enter", validate, onchange, testid }: Props = $props();
  let text = $state("");
  let err = $state<string | null>(null);
  const id = `ci${Math.random().toString(36).slice(2, 8)}`;

  function add() {
    const v = text.trim().replace(/,$/, "").trim();
    if (!v) return;
    const e = validate?.(v) ?? null;
    if (e) {
      err = e;
      return;
    }
    if (!values.includes(v)) {
      values = [...values, v];
      onchange?.(values);
    }
    text = "";
    err = null;
  }
  function remove(v: string) {
    values = values.filter((x) => x !== v);
    onchange?.(values);
  }
</script>

<div data-testid={testid}>
  <label class="label" for={id}>{label}</label>
  <div class="input flex h-auto min-h-8 flex-wrap items-center gap-1 py-1" aria-invalid={err ? "true" : undefined}>
    {#each values as v (v)}
      <span class="badge h-6 gap-1 pr-1">
        <span class="max-w-48 truncate">{v}</span>
        <button type="button" class="rounded hover:bg-black/10" aria-label="Remove {v}" onclick={() => remove(v)}><X size={12} /></button>
      </span>
    {/each}
    <input
      {id}
      class="min-w-28 flex-1 bg-transparent text-[13px] outline-none"
      {placeholder}
      bind:value={text}
      onkeydown={(e) => {
        if (e.key === "Enter" || e.key === ",") {
          e.preventDefault();
          add();
        } else if (e.key === "Backspace" && !text && values.length) {
          remove(values[values.length - 1] as string);
        }
      }}
      onblur={add}
    />
  </div>
  {#if err}<p class="mt-1 text-xs text-danger" role="alert">{err}</p>{/if}
</div>
