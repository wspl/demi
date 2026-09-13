<script setup lang="ts">
import { computed } from 'vue'
import { GitCompareArrows, PanelRightClose, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import TabStrip from './TabStrip.vue'
import { workTabTitle, type WorkTab } from './work-panel'

/**
 * The work panel: the app frame's right pane, where the reader keeps files
 * and diffs open beside the conversation. A raised pane like the session's,
 * with the tab row where the session has its header, so the two panes line
 * up across the divider. The host owns the tabs and which one is active; the
 * panel only shows them.
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
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden rounded-tl-xl bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <TabStrip
        class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto"
        role="tablist"
      >
        <span
          v-for="tab in tabs"
          :key="tab.id"
          role="tab"
          :aria-selected="tab.id === activeId"
          class="group relative flex h-7 shrink-0 cursor-default select-none items-center overflow-hidden rounded-md pl-1.5 pr-7 text-chrome"
          :class="
            tab.id === activeId
              ? 'bg-surface-base text-fg-emphasis'
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
            class="max-w-28 truncate whitespace-nowrap px-1.5"
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
