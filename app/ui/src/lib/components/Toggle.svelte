<script lang="ts">
  interface Props {
    checked: boolean;
    label: string;
    hideLabel?: boolean;
    disabled?: boolean;
    description?: string;
    onchange?: (v: boolean) => void;
  }
  let { checked = $bindable(), label, hideLabel = false, disabled = false, description, onchange }: Props = $props();
  const id = `t${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="flex items-start gap-3">
  <button
    {id}
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    {disabled}
    class="relative mt-0.5 inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border border-transparent transition-colors disabled:cursor-not-allowed disabled:opacity-50 {checked
      ? 'bg-accent'
      : 'bg-surface-3'}"
    onclick={() => {
      // Controlled when `onchange` is given (the parent decides); otherwise bindable.
      if (onchange) onchange(!checked);
      else checked = !checked;
    }}
  >
    <span class="inline-block h-4 w-4 rounded-full bg-white shadow transition-transform {checked ? 'translate-x-4' : 'translate-x-0.5'}"></span>
  </button>
  {#if !hideLabel}
    <div class="min-w-0">
      <label for={id} class="cursor-pointer text-[13px] font-medium">{label}</label>
      {#if description}<p class="text-xs text-muted">{description}</p>{/if}
    </div>
  {/if}
</div>
