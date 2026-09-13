<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { GitCompareArrows, PanelRightClose, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import TabStrip from './TabStrip.vue'
import { workTabTitle, type WorkTab } from './work-panel'

/**
 * The work panel: the app frame's right pane, where the reader keeps files
 * and diffs open beside the conversation. It continues the session's raised
 * sheet behind a hairline divider, with the tab row at the height of the
 * session header. The host owns the tabs and which one is active; the panel
 * only shows them.
 *
 * The content pane is a placeholder until file and diff views exist.
 */
const props = defineProps<{
  tabs: readonly WorkTab[]
  activeId: string | null
}>()
const emit = defineEmits<{
  select: [id: string]
  closeTab: [id: string]
  /** The fold control: put the whole panel away. */
  close: []
}>()
const active = computed(
  () => props.tabs.find((tab) => tab.id === props.activeId) ?? null,
)

// Tabs are all one width; when they outgrow the row the strip scrolls with
// no scrollbar and fades out at whichever edge has more behind it.
const strip = ref<{ el: HTMLElement | null } | null>(null)
const moreBefore = ref(false)
const moreAfter = ref(false)
let observer: ResizeObserver | null = null

function updateEdges(): void {
  const el = strip.value?.el
  if (!el) {
    return
  }
  moreBefore.value = el.scrollLeft > 0
  moreAfter.value = el.scrollLeft + el.clientWidth < el.scrollWidth - 1
}

function revealActive(): void {
  const el = strip.value?.el
  const tab = el?.querySelector<HTMLElement>('[aria-selected="true"]')
  tab?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
  updateEdges()
}

onMounted(() => {
  const el = strip.value?.el
  if (el) {
    observer = new ResizeObserver(updateEdges)
    observer.observe(el)
  }
  revealActive()
})
onBeforeUnmount(() => {
  observer?.disconnect()
})
watch(
  () => [props.tabs.length, props.activeId],
  () => {
    void nextTick(revealActive)
  },
)
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <div class="relative min-w-0 flex-1">
        <TabStrip
          ref="strip"
          class="flex items-center gap-0.5 overflow-x-auto [scrollbar-width:none]"
          role="tablist"
          @scroll.passive="updateEdges"
        >
          <span
            v-for="tab in tabs"
            :key="tab.id"
            role="tab"
            :aria-selected="tab.id === activeId"
            class="group relative flex h-7 w-36 shrink-0 cursor-default select-none items-center overflow-hidden rounded-md pl-1.5 pr-7 text-chrome after:absolute after:right-0 after:top-1/2 after:h-3.5 after:w-px after:-translate-y-1/2 after:bg-line last:after:hidden hover:after:hidden has-[+[aria-selected=true]]:after:hidden"
            :class="
              tab.id === activeId
                ? 'bg-surface-base text-fg-emphasis after:hidden'
                : 'text-fg-subtle hover:bg-hover hover:text-fg-body'
            "
            @pointerdown="emit('select', tab.id)"
          >
            <span class="relative flex shrink-0 items-center">
              <FileIcon :name="workTabTitle(tab)" :is-directory="false" :size="ICON_PX.markIn28" />
              <GitCompareArrows
                v-if="tab.kind === 'diff'"
                :size="8"
                class="absolute -bottom-0.5 -right-0.5 rounded-full bg-surface text-fg-muted"
              />
            </span>
            <Tooltip
              :content="tab.path"
              placement="bottom"
              class="min-w-0 flex-1 truncate whitespace-nowrap px-1.5"
              >{{ workTabTitle(tab) }}</Tooltip
            >
            <span
              role="button"
              aria-label="Close"
              class="absolute right-1 top-1 flex size-5 items-center justify-center rounded text-fg-faint opacity-0 transition-[opacity,color,background-color] hover:bg-hover hover:text-fg-body group-hover:opacity-100"
              :class="tab.id === activeId && 'opacity-100'"
              @pointerdown.stop
              @click.stop="emit('closeTab', tab.id)"
            >
              <X :size="ICON_PX.in20" />
            </span>
          </span>
        </TabStrip>
        <div
          v-if="moreBefore"
          aria-hidden="true"
          class="pointer-events-none absolute inset-y-0 left-0 w-6 bg-linear-to-r from-surface to-surface/0"
        />
        <div
          v-if="moreAfter"
          aria-hidden="true"
          class="pointer-events-none absolute inset-y-0 right-0 w-6 bg-linear-to-l from-surface to-surface/0"
        />
      </div>
      <IconButton
        :icon="PanelRightClose"
        size="sm"
        variant="ghost"
        aria-label="Close panel"
        @click="emit('close')"
      />
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <slot :tab="active">
        <div
          class="flex flex-1 select-none flex-col items-center justify-center gap-1 text-[13px] text-fg-faint"
        >
          <template v-if="active">
            <span>{{ active.kind === 'diff' ? 'Diff' : 'File' }}</span>
            <span class="max-w-full truncate px-4 font-mono text-[11px]">{{ active.path }}</span>
          </template>
          <span v-else>No files open</span>
        </div>
      </slot>
    </div>
  </aside>
</template>
