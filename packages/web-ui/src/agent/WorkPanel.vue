<script setup lang="ts">
import { computed } from 'vue'
import { GitCompareArrows, PanelRightClose } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import { ICON_PX } from '../ui/icon-metrics'
import FileIcon from '../files/FileIcon.vue'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import { workTabTitle, type WorkTab } from './work-panel'

/**
 * The work panel: the app frame's right pane, where the reader keeps files
 * and diffs open beside the conversation. It continues the session's raised
 * sheet behind a hairline divider, with the tab row (on the raised surface)
 * at the height of the session header. The host owns the tabs and which one
 * is active; the panel only shows them.
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
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 px-2">
      <TabStrip class="flex-1" surface="raised">
        <TabItem
          v-for="tab in tabs"
          :key="tab.id"
          :tab="{ id: tab.id, title: workTabTitle(tab) }"
          :tooltip="tab.path"
          :is-active="tab.id === activeId"
          @pointerdown="emit('select', tab.id)"
          @close="emit('closeTab', tab.id)"
        >
          <template #mark>
            <FileIcon :name="workTabTitle(tab)" :is-directory="false" :size="ICON_PX.markIn28" />
            <GitCompareArrows
              v-if="tab.kind === 'diff'"
              :size="8"
              class="absolute -bottom-0.5 -right-0.5 rounded-full bg-surface text-fg-muted"
            />
          </template>
        </TabItem>
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
