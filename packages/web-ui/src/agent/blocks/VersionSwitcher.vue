<script setup lang="ts">
import { ChevronLeft, ChevronRight } from '@lucide/vue'
import { t } from '../locales/text'
import type { VersionChoice } from '../versions'
import IconButton from '../../ui/IconButton.vue'
import Tooltip from '../../ui/Tooltip.vue'

/**
 * ‹ 2/3 › under an edited message and in a regenerated answer's footer, as
 * ChatGPT and Claude show it: the arrows show the version before and
 * after, and wait while the agent works.
 */
defineProps<{
  choice: VersionChoice
}>()
</script>

<template>
  <div
    class="inline-flex select-none items-center text-fg-faint phone:gap-2"
    role="group"
    :aria-label="t('transcript.versions.position', { index: choice.index, count: choice.count })"
  >
    <Tooltip :content="t('transcript.versions.previous')" :disabled="choice.disabled || choice.index <= 1">
      <IconButton
        :icon="ChevronLeft"
        size="sm"
        variant="ghost"
        :aria-label="t('transcript.versions.previous')"
        :disabled="choice.disabled || choice.index <= 1"
        @click.stop="choice.choose(choice.index - 1)"
      />
    </Tooltip>
    <span class="min-w-6 text-center text-caption leading-6 tabular-nums phone:leading-9">{{ choice.index }}/{{ choice.count }}</span>
    <Tooltip :content="t('transcript.versions.next')" :disabled="choice.disabled || choice.index >= choice.count">
      <IconButton
        :icon="ChevronRight"
        size="sm"
        variant="ghost"
        :aria-label="t('transcript.versions.next')"
        :disabled="choice.disabled || choice.index >= choice.count"
        @click.stop="choice.choose(choice.index + 1)"
      />
    </Tooltip>
  </div>
</template>
