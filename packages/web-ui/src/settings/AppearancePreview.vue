<script setup lang="ts">
import { Check } from '@lucide/vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'

/**
 * What the appearance rows change, shown as itself rather than as a miniature app:
 * a passage at the chosen text size, the tone's stack of surfaces, and the controls
 * that take the accent. Everything reads the live tokens, so it follows each choice.
 */
defineProps<{
  /** Transcript text size in px. */
  fontSize?: number
}>()
</script>

<template>
  <div class="flex aspect-[3/4] w-56 select-none flex-col gap-3 rounded-xl border border-line bg-surface p-4 text-fg" aria-hidden="true">
    <!-- Text at size -->
    <div class="flex min-h-0 flex-1 flex-col gap-1.5">
      <div class="font-medium leading-none text-fg-emphasis" :style="{ fontSize: `${(fontSize ?? 15) * 1.6}px` }">Aa</div>
      <p class="line-clamp-4 text-fg-body" :style="{ fontSize: `${fontSize ?? 15}px`, lineHeight: 1.5 }">
        The quick brown fox jumps over the lazy dog while the model streams its reply.
      </p>
    </div>
    <!-- Tone: the surfaces stacked -->
    <div class="flex gap-1.5">
      <div class="h-12 flex-1 rounded-md bg-surface-base ring-1 ring-line-subtle" />
      <div class="h-12 flex-1 rounded-md bg-surface ring-1 ring-line-subtle" />
      <div class="h-12 flex-1 rounded-md bg-surface-raised ring-1 ring-line-subtle" />
      <div class="h-12 flex-1 rounded-md bg-surface-float ring-1 ring-line-subtle" />
    </div>
    <!-- Accent: the controls that carry it -->
    <div class="flex items-center gap-2">
      <span class="btn-primary inline-flex h-6 items-center gap-1 rounded-md px-2 text-[12px] font-medium text-white"><Check :size="12" /> Send</span>
      <Tag tone="accent">Selected</Tag>
      <Switch :model-value="true" size="sm" class="ml-auto" />
    </div>
    <div class="flex h-7 items-center rounded-md bg-surface-raised px-2 text-[12px] text-fg-subtle ring-1 ring-line-focus">Focused</div>
  </div>
</template>
