<template>
  <div class="flex items-center px-[var(--agent-pad-x,2rem)] py-2">
    <div class="loading-dots flex gap-1">
      <span class="size-1.5 rounded-full bg-fg-subtle" />
      <span class="size-1.5 rounded-full bg-fg-subtle" />
      <span class="size-1.5 rounded-full bg-fg-subtle" />
    </div>
  </div>
</template>

<style scoped>
.loading-dots span {
  animation: dot-pulse 1.4s ease-in-out infinite;
}

.loading-dots span:nth-child(2) {
  animation-delay: 0.2s;
}

.loading-dots span:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes dot-pulse {
  0%, 80%, 100% { opacity: 0.3; }
  40% { opacity: 1; }
}
</style>
