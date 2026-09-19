<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { FolderTree } from '@lucide/vue'
import { useElementSize } from '@vueuse/core'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import Tooltip from '../ui/Tooltip.vue'
import { CONTENT_MIN_WIDTH, TREE_WIDTH } from './file-view'

/**
 * The frame a file view sits in: a header row, and under it the view with a
 * tree at its end. While the view keeps `CONTENT_MIN_WIDTH` beside it, the
 * tree docks there behind a divider that sizes it, and the control at the
 * header's end shows and hides it; the host keeps both (v-model `open` and
 * `width`). A frame too narrow for that hides the tree on its own, and the
 * control then shows it over the view at the same width, until the control,
 * a click on the view beside it or a pick in it (`dismiss`) puts it away.
 * Once the frame is wide enough again, the tree docks if `open`.
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

function toggle(): void {
  if (docks.value)
    open.value = !open.value
  else
    over.value = !over.value
}

/** Shows the tree: docked where it fits, otherwise over the view. */
function show(): void {
  if (docks.value)
    open.value = true
  else
    over.value = true
}

/** Puts away a tree shown over the view, as a pick in it should; a docked tree stays. */
function dismiss(): void {
  over.value = false
}

defineExpose({ show, dismiss })
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
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
    </div>
    <div ref="body" class="relative flex min-h-0 flex-1 border-t border-line">
      <!-- Its own stacking context: nothing in the view rises over a tree shown above it. -->
      <div class="relative isolate min-w-0 flex-1">
        <slot />
      </div>
      <template v-if="$slots.tree && shown">
        <ResizeHandle
          v-if="docks"
          v-model="width"
          side="end"
          :min="TREE_WIDTH.min"
          :max="room"
          :default-value="TREE_WIDTH.default"
          :label="name"
        />
        <!-- Behind a tree shown over the view: a click beside the tree puts it away. -->
        <div v-else class="absolute inset-0 z-10 bg-black/50" @click="dismiss" />
        <!-- The tree's width is the divider's; flex must not grow or shrink it. -->
        <div
          class="border-l border-line"
          :class="docks ? '' : 'absolute inset-y-0 right-0 z-10 max-w-full'"
          :style="{ flex: `0 0 ${width}px`, width: `${width}px` }"
        >
          <slot name="tree" />
        </div>
      </template>
    </div>
  </div>
</template>
