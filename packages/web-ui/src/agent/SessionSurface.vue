<script setup lang="ts">
import { computed, provide, ref } from 'vue'
import { useElementSize } from '@vueuse/core'
import DockWindowArea from './DockWindowArea.vue'
import { DOCK_WINDOW_SHARE } from './dock-window'
import SessionBottomVeil from './SessionBottomVeil.vue'
import { COMPOSER_VEIL_FADE_PX } from './composer-clearance'
import { sessionComposerKey, sessionCoverKey } from './session-cover'

/**
 * The share of this area's height the dock's windows keep (`dock-window.ts`);
 * the host keeps it.
 */
const dockWindowShare = defineModel<number>('dockWindowShare', { default: DOCK_WINDOW_SHARE.default })
const emit = defineEmits<{
  /** The share of a dock window settled by a drag, a key or a double-click. */
  dockWindowResizeEnd: [share: number]
}>()

const surfaceRef = ref<HTMLDivElement>()
const { height: areaHeight } = useElementSize(surfaceRef, { width: 0, height: 0 }, { box: 'border-box' })
const bottomAreaRef = ref<HTMLDivElement>()
// Border-box: the dock's full footprint, its own padding included.
const { height: dockFootprint } = useElementSize(
  bottomAreaRef,
  { width: 0, height: 0 },
  { box: 'border-box' }
)
/**
 * What stands over the dock's top, such as a permission card; the dock sets it.
 */
const coverHeight = ref(0)
provide(sessionCoverKey, coverHeight)
/** The dock's composer, under its chips row; the dock sets it. */
const composerHeight = ref(0)
provide(sessionComposerKey, composerHeight)
const dockRef = ref<HTMLDivElement>()
const { height: dockOwnHeight } = useElementSize(dockRef, { width: 0, height: 0 }, { box: 'border-box' })
/**
 * How high the veil reaches from the area's foot: the padding under the dock,
 * the composer, and the veil's fade above it (`SessionBottomVeil`).
 */
const veilReach = computed(() =>
  composerHeight.value > 0
    ? dockFootprint.value - dockOwnHeight.value + composerHeight.value + COMPOSER_VEIL_FADE_PX
    : 0,
)
/**
 * What the transcript keeps clear at its foot: the dock and what stands over
 * it, or the veil where it reaches higher. A panel the dock opened stands on
 * the dock alone, so the card covers its lower part and it never moves when
 * a card comes or goes.
 */
const dockHeight = computed(() => Math.max(dockFootprint.value + coverHeight.value, veilReach.value))

defineExpose({ dockHeight })
</script>

<template>
  <!--
    The conversation reads in one column at most 50rem wide, its transcript and
    its dock alike. The space either side, --session-gutter, is padding inside
    the full-width scroller, so the scrollbar stays at the edge.
  -->
  <div
    ref="surfaceRef"
    class="relative h-full flex-1 overflow-hidden bg-surface [container-type:inline-size]"
    style="--session-gutter: max(0px, calc((100cqw - 50rem) / 2))"
  >
    <slot />
    <!--
      The dock's windows stand on the dock, in the room above it up to the
      session's top.
    -->
    <DockWindowArea
      v-if="$slots.overDock"
      v-model:share="dockWindowShare"
      class="absolute inset-x-[calc(var(--session-gutter)+0.75rem)] top-0 z-20"
      :style="{ bottom: `${dockFootprint}px` }"
      :area="areaHeight"
      @resize-end="emit('dockWindowResizeEnd', $event)"
    >
      <slot name="overDock" />
    </DockWindowArea>
    <!--
      No stacking context of its own: the veil stays under a panel the dock
      opened, and the dock, with what stands over its top, above it.
    -->
    <div
      ref="bottomAreaRef"
      class="absolute bottom-0 left-0 right-0 px-[calc(var(--session-gutter)+0.75rem)] pb-[max(0.75rem,var(--safe-bottom))]"
    >
      <SessionBottomVeil :composer="composerHeight" />
      <div ref="dockRef" class="relative z-30">
        <slot name="dock" />
      </div>
    </div>
  </div>
</template>
