<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { FolderTree } from '@lucide/vue'
import { useElementSize, usePreferredReducedMotion } from '@vueuse/core'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Tooltip from '../ui/Tooltip.vue'
import TooltipPlacement from '../ui/TooltipPlacement.vue'
import { CONTENT_MIN_WIDTH, TREE_MOTION_MS, TREE_WIDTH } from './file-view'

/**
 * The frame a file view sits in: a header row, and under it the view with a
 * tree at its end. While the view keeps `CONTENT_MIN_WIDTH` beside it, the
 * tree docks there behind a divider that sizes it, and the control at the
 * header's end shows and hides it; the host keeps both (v-model `open` and
 * `width`). A frame too narrow for that hides the tree on its own, and the
 * control then shows it over the view at the same width, until the control,
 * a click on the view beside it or a pick in it (`dismiss`) puts it away.
 * Once the frame is wide enough again, the tree docks if `open`. Showing
 * and hiding by the frame's own control, or by a pick or a click beside a
 * tree over the view, move: a docked tree grows from the end and the view
 * gives way, and a tree over the view slides in from the end. Anything else
 * that shows or hides the tree, such as the frame showing again in a work
 * panel tab selected again, or its width becoming known, shows it at once.
 * A hidden tree stays mounted, so it shows again as it was left, with no
 * reading.
 */
defineProps<{
  /** The tree in its control's words: `file tree` reads "Show file tree". */
  name: string
}>()

defineSlots<{
  /** The header row, before the tree's control. */
  header(): unknown
  /** The view. */
  default(): unknown
  /** The tree; without it there is no tree and no control. */
  tree?(): unknown
}>()

const open = defineModel<boolean>('open', { default: true })
const width = defineModel<number>('width', { default: TREE_WIDTH.default })

const body = ref<HTMLElement | null>(null)
const { width: bodyWidth } = useElementSize(body)
/** The widest the tree can dock at: what the view leaves it. */
const room = computed(() => Math.min(TREE_WIDTH.max, Math.floor(bodyWidth.value - CONTENT_MIN_WIDTH)))
const docks = computed(() => width.value <= room.value)
/** While the tree cannot dock: whether it is shown over the view. */
const over = ref(false)
const shown = computed(() => docks.value ? open.value : over.value)

// A tree shown over the view does not come back by itself when the frame narrows again.
watch(docks, () => {
  over.value = false
})

/** Whether the tree's next show or hide is the user's, which moves. */
let requested = false

/** Runs the user's `change` of the tree, whose show or hide moves; the flag ends with the render it causes. */
function byUser(change: () => void): void {
  requested = true
  change()
  void nextTick(() => {
    requested = false
  })
}

function toggle(): void {
  byUser(() => {
    if (docks.value)
      open.value = !open.value
    else
      over.value = !over.value
  })
}

/** Shows the tree: docked where it fits, otherwise over the view. */
function show(): void {
  byUser(() => {
    if (docks.value)
      open.value = true
    else
      over.value = true
  })
}

/** Puts away a tree shown over the view, as a pick in it should; a docked tree stays. */
function dismiss(): void {
  byUser(() => {
    over.value = false
  })
}

const motion = usePreferredReducedMotion()

/**
 * The tree's keyframes from hidden to shown: a docked tree grows its width
 * from nothing, which the view beside it gives up; one over the view slides
 * in from the end.
 */
function shownFrames(): Keyframe[] {
  if (docks.value) {
    const docked = `${width.value}px`
    return [
      { flexBasis: '0px', width: '0px' },
      { flexBasis: docked, width: docked },
    ]
  }
  return [{ transform: 'translateX(100%)' }, { transform: 'none' }]
}

/** The tree's move under way, which a show or hide in its midst turns around. */
let moving: Animation | null = null

/**
 * Moves the tree to shown, or back to hidden, and ends the transition when it
 * is there. A move that turns another around starts where that one was: the
 * reversed curve is the same at the mirrored time.
 */
function move(element: Element, direction: PlaybackDirection, done: () => void): void {
  const turned = moving?.currentTime
  // The turned move's transition was cancelled before this one began, so its end is never reported.
  moving?.cancel()
  moving = null
  if (motion.value === 'reduce' || !requested) {
    done()
    return
  }
  const animation = element.animate(shownFrames(), {
    duration: TREE_MOTION_MS,
    easing: 'ease-out',
    direction,
  })
  if (typeof turned === 'number') {
    animation.currentTime = TREE_MOTION_MS - turned
  }
  moving = animation
  // A cancelled move rejects; whoever cancelled it has its own transition under way.
  animation.finished.then(
    () => {
      moving = null
      done()
    },
    () => {},
  )
}

defineExpose({ show, dismiss })
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <!-- The row stands right under the panel's tab strip: its tips open below, never over the strip. -->
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <TooltipPlacement placement="bottom">
        <slot name="header" />
        <Tooltip v-if="$slots.tree" :content="shown ? `Hide ${name}` : `Show ${name}`" class="shrink-0">
          <IconButton
            :icon="FolderTree"
            variant="ghost"
            :pressed="shown"
            :aria-label="shown ? `Hide ${name}` : `Show ${name}`"
            @click="toggle"
          />
        </Tooltip>
      </TooltipPlacement>
    </div>
    <!-- It clips a tree on its way in or out at the end. -->
    <div ref="body" class="relative flex min-h-0 flex-1 overflow-hidden border-t border-line">
      <!-- Its own stacking context: nothing in the view rises over a tree shown above it. -->
      <div class="relative isolate min-w-0 flex-1">
        <slot />
      </div>
      <template v-if="$slots.tree">
        <ResizeHandle
          v-if="shown && docks"
          v-model="width"
          side="end"
          :min="TREE_WIDTH.min"
          :max="room"
          :default-value="TREE_WIDTH.default"
          :label="name"
        />
        <!-- Behind a tree shown over the view: a click beside the tree puts it away. The view stays as it is,
             as beside an open menu; the tree's shadow sets it above. -->
        <div v-else-if="shown" class="absolute inset-0 z-10" @click="dismiss" />
        <Transition
          :css="false"
          @enter="(element, done) => move(element, 'normal', done)"
          @leave="(element, done) => move(element, 'reverse', done)"
        >
          <!-- The tree's width is the divider's; flex must not grow or shrink it. While a docked tree grows or
               shrinks, its content keeps that width and the frame cuts it. -->
          <div
            v-show="shown"
            class="overflow-hidden border-l border-line"
            :class="docks ? '' : 'absolute inset-y-0 right-0 z-10 max-w-full shadow-lg'"
            :style="{ flex: `0 0 ${width}px`, width: `${width}px` }"
          >
            <div class="h-full" :class="docks ? '' : 'w-full'" :style="docks ? { width: `${width}px` } : undefined">
              <slot name="tree" />
            </div>
          </div>
        </Transition>
      </template>
    </div>
  </div>
</template>
