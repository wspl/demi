<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { Network, TriangleAlert, UserRoundX, Workflow } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import Tooltip from '../ui/Tooltip.vue'
import { useIconPx } from '../ui/icon-metrics'
import { workflowRows, workflowShown, type WorkflowChoice, type WorkflowList } from './workflow-menu'

/**
 * The composer's Workflow menu at the row's left end: it shows the
 * conversation's choice and offers Global Workflow, No Subagents and the
 * project's workflows by name. A workflow that does not read, or the chosen one
 * once it is gone, stands in the menu with why and cannot be chosen. Where the
 * row has no room (`compact`), it keeps its mark alone, and its tip reads the
 * choice.
 */
const props = defineProps<{
  choice: WorkflowChoice
  list: WorkflowList
  compact?: boolean
}>()

const emit = defineEmits<{ change: [choice: WorkflowChoice] }>()

const icons = useIconPx()
const shown = computed(() => workflowShown(props.choice, props.list))
const rows = computed(() => workflowRows(props.choice, props.list))
const fixed = computed(() => rows.value.slice(0, 2))
const listed = computed(() => rows.value.slice(2))
const tip = computed(() => shown.value.unavailable ?? (props.compact ? t('workflows.labelTip', { name: shown.value.label }) : null))

function choose(choice: WorkflowChoice | null, close: () => void) {
  close()
  if (choice)
    emit('change', choice)
}
</script>

<template>
  <Dropdown
    :overlay-store="appOverlayStore"
    variant="ghost"
    side="top"
    :trigger-label="t('workflows.label')"
  >
    <template #trigger="{ isOpen }">
      <Tooltip :content="tip ?? ''" :disabled="!tip">
        <span class="relative inline-flex items-center overflow-hidden">
          <Network :size="icons.chip" class="mr-1 shrink-0" />
          <span
            v-if="!compact"
            class="whitespace-nowrap"
            :class="isOpen ? 'text-fg-body' : 'text-fg-muted'"
          >{{ shown.label }}</span>
          <TriangleAlert
            v-if="shown.unavailable"
            :size="icons.chip"
            class="ml-1 shrink-0 text-on-warning"
          />
        </span>
      </Tooltip>
    </template>
    <template #content="{ close }">
      <Menu>
        <MenuItem
          v-for="row in fixed"
          :key="row.key"
          :icon="row.key === 'none' ? UserRoundX : Network"
          :label="row.label"
          choice
          :is-selected="row.selected"
          @select="choose(row.choice, close)"
        />
        <template v-if="listed.length">
          <MenuDivider />
          <MenuItem
            v-for="row in listed"
            :key="row.key"
            :icon="Workflow"
            :label="row.label"
            choice
            :is-selected="row.selected"
            :disabled="row.unavailable !== null || row.choice === null"
            :disabled-reason="row.unavailable ?? undefined"
            @select="choose(row.choice, close)"
          />
        </template>
      </Menu>
    </template>
  </Dropdown>
</template>
