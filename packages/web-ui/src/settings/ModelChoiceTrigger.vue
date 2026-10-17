<script setup lang="ts">
import { TriangleAlert, Zap } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import { t } from './locales/text'
import type { ModelChoiceLabel } from './model-choice'

/**
 * The face of a settings row's model menu, a subagent profile's or the
 * small model's: the warning while the choice is missing a part, the model,
 * its effort a step quieter, and the bolt while Fast is on.
 */
defineProps<{
  label: ModelChoiceLabel
  /** Why the choice cannot be used as it is; null while it can. */
  missing?: string | null
}>()
</script>

<template>
  <span class="flex min-w-0 items-center gap-1.5">
    <TriangleAlert v-if="missing" :size="ICON_PX.in24" class="shrink-0 text-on-warning" :aria-label="missing" />
    <span class="flex min-w-0 items-center gap-1">
      <!-- Clipped, not hidden, so the name keeps its text baseline beside an inline detail (InlineDetail). -->
      <span class="min-w-0 overflow-clip text-ellipsis whitespace-nowrap">{{ label.name }}</span>
      <span v-if="label.effort" class="shrink-0 text-fg-subtle">{{ label.effort }}</span>
      <Zap v-if="label.fast" :size="ICON_PX.in24" class="shrink-0" :aria-label="t('profile.fast')" />
    </span>
  </span>
</template>
