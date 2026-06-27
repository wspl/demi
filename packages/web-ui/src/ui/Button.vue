<script setup lang="ts">
defineProps<{
  size?: 'xs' | 'sm' | 'md'
  disabled?: boolean
}>()
</script>

<template>
  <span
    class="btn inline-flex cursor-pointer items-center justify-center gap-1 rounded-md font-medium text-fg-body transition-all select-none hover:text-fg-emphasis active:translate-y-px active:brightness-90"
    :class="[
      size === 'md' ? 'px-3 py-1 text-[12px]' : size === 'xs' ? 'px-1.5 py-0.5 text-[11px]' : 'px-2.5 py-0.5 text-[11px]',
      disabled ? 'pointer-events-none opacity-40' : '',
    ]"
  >
    <slot />
  </span>
</template>

<style scoped>
.btn {
  background: var(--btn-bg);
  box-shadow: var(--shadow-btn);
}

.btn:hover {
  background: var(--btn-bg-hover);
  box-shadow: var(--shadow-btn-hover);
}
</style>
