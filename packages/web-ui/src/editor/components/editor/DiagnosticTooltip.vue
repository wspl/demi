<script setup lang="ts">
defineProps<{
  message: string
  severity: 'error' | 'warning' | 'info' | 'hint'
  source?: string
}>()

const severityColors: Record<string, string> = {
  error: 'text-on-danger',
  warning: 'text-on-warning',
  info: 'text-on-accent',
  hint: 'text-fg-muted',
}

const severityLabels: Record<string, string> = {
  error: 'Error',
  warning: 'Warning',
  info: 'Info',
  hint: 'Hint',
}
</script>

<template>
  <div class="max-w-[500px] p-2 text-xs leading-relaxed">
    <div class="flex items-baseline gap-1.5">
      <span :class="severityColors[severity]" class="shrink-0 text-[11px] font-medium">{{ severityLabels[severity] }}</span>
      <span v-if="source" class="shrink-0 text-fg-subtle">{{ source }}</span>
    </div>
    <div class="mt-1 text-fg">{{ message }}</div>
  </div>
</template>
