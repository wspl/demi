<script setup lang="ts">
import { computed } from 'vue'
import { RotateCcw } from '@lucide/vue'
import { IconButton, Slider } from '@demicodes/plugin-sdk'
import { t } from '../locales/text'

/**
 * The capsule under a hinged foldable, on the view's own background: the
 * hinge's slider with its angle, and Reset, which faces the device again.
 * The postures are the toolbar's. It keeps its room in every posture.
 */
const props = defineProps<{
  /** The hinge's angle the view shows, in degrees. */
  degrees: number
  /** Whether the view is turned from facing the device. */
  turned: boolean
}>()

const emit = defineEmits<{
  /** The slider moves the hinge to an angle. */
  hinge: [degrees: number]
  reset: []
}>()

const angle = computed(() => Math.round(props.degrees))
</script>

<template>
  <div class="flex shrink-0 justify-center px-3 pb-3">
    <div class="flex h-9 w-full max-w-90 items-center gap-2 rounded-full border border-line bg-surface pl-4 pr-1 shadow-sm">
      <Slider
        class="min-w-0 flex-1"
        :model-value="angle"
        :min="0"
        :max="180"
        :value-label="t('fold.degrees', { degrees: angle })"
        :aria-label="t('fold.hinge')"
        @update:model-value="emit('hinge', $event)"
      />
      <IconButton
        :icon="RotateCcw"
        size="sm"
        variant="ghost"
        class="rounded-full"
        :disabled="!turned"
        :aria-label="t('fold.reset')"
        @click="emit('reset')"
      />
    </div>
  </div>
</template>
