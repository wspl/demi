<script setup lang="ts">
import { computed } from 'vue'
import { RotateCcw } from '@lucide/vue'
import { IconButton, Slider } from '@demicodes/plugin-sdk'
import { t } from '../locales/text'

/**
 * The bar under a hinged foldable, on the view's own background: a capsule
 * with the hinge's slider and its angle, and beside it, in a circle of the
 * same look, Reset, which faces the device again. The postures are the
 * toolbar's. It keeps its room in every posture.
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
  <div class="flex shrink-0 justify-center gap-2 px-3 pb-3">
    <div class="flex h-9 min-w-0 max-w-80 flex-1 items-center rounded-full border border-line bg-surface px-4 shadow-sm">
      <Slider
        class="min-w-0 flex-1"
        :model-value="angle"
        :min="0"
        :max="180"
        :value-label="t('fold.degrees', { degrees: angle })"
        :aria-label="t('fold.hinge')"
        @update:model-value="emit('hinge', $event)"
      />
    </div>
    <!-- The circle's ring around the button resets too, so all of the circle the user sees takes the click. -->
    <div
      class="flex size-9 shrink-0 items-center justify-center rounded-full border border-line bg-surface shadow-sm"
      @click.self="turned && emit('reset')"
    >
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
