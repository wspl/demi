<script setup lang="ts">
import { computed, provide, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import { DOCK_WINDOW_SHARE, dockWindowKey } from './dock-window'

/**
 * The space a dock window (`SessionOverlay`) stands in: the box it fills, with
 * the room its top edge's handle needs above it and the dock's step below.
 * It tells the window how tall it may be and keeps the share of the area's
 * height the window is: `v-model:share` is live while the edge drags, and
 * `resizeEnd` says the share has settled, for the host to keep.
 *
 * `area` is the height the share is of; a surface that stands over a larger
 * session area gives that area's, and without it the box itself is the area.
 * The box takes its place and size from the caller.
 */
const props = defineProps<{
  area?: number
}>()

const share = defineModel<number>('share', { default: DOCK_WINDOW_SHARE.default })

const emit = defineEmits<{
  resizeEnd: [share: number]
}>()

const root = ref<HTMLElement>()
const room = ref<HTMLElement>()
const { height: ownHeight } = useElementSize(root, { width: 0, height: 0 }, { box: 'border-box' })
const { height: roomHeight } = useElementSize(room, { width: 0, height: 0 }, { box: 'border-box' })

provide(dockWindowKey, {
  area: computed(() => props.area ?? ownHeight.value),
  room: roomHeight,
  share,
  commit: (settled) => emit('resizeEnd', settled),
})
</script>

<template>
  <!-- The 4px above and the 8px below are the handle's half reach over the window's top edge and
       the step the dock keeps from its chips; the window stands in the box between them. -->
  <div ref="root" class="pointer-events-none pb-2 pt-1">
    <!-- A window here is in the column already: a transcript inside it takes no gutter again. -->
    <div ref="room" class="relative h-full min-h-0 [--session-gutter:0px]">
      <slot />
    </div>
  </div>
</template>
