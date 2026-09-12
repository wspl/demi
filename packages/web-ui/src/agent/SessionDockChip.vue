<script setup lang="ts">
withDefaults(defineProps<{
  dot?: 'accent' | 'success' | 'muted'
  breathing?: boolean
}>(), {})
</script>

<template>
  <span
    role="button"
    class="btn inline-flex h-7 cursor-default select-none items-center gap-1.5 rounded-full px-2.5 text-chrome text-fg-body hover:text-fg-emphasis"
  >
    <span
      v-if="dot"
      class="size-1.5 shrink-0 rounded-full"
      :class="[
        dot === 'accent' ? 'bg-on-accent' : dot === 'success' ? 'bg-on-success' : 'bg-fg-subtle',
        { 'dock-dot-breathing': breathing },
      ]"
    />
    <slot />
  </span>
</template>

<style scoped>
@keyframes dock-dot-breathe {
  0%,
  100% {
    opacity: 0.35;
    transform: scale(0.85);
  }

  50% {
    opacity: 1;
    transform: scale(1.1);
  }
}

.dock-dot-breathing {
  animation: dock-dot-breathe 2.4s ease-in-out infinite;
}

@media (prefers-reduced-motion: reduce) {
  .dock-dot-breathing {
    animation: none;
  }
}
</style>
