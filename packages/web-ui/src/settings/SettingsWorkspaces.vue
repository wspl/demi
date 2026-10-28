<script setup lang="ts">
/**
 * Settings › Workspaces: the account's workspaces beside the one selected,
 * as Providers reads.
 *
 * - The list is in the switcher's order, which dragging a row or Alt and an
 *   arrow key change. New above it makes one at once and selects it, its
 *   name selected in the field, ready to type over.
 * - The selected one shows its name over Delete; below, its name in a field,
 *   its tile, which opens the colors and the icons, and how many
 *   conversations and projects it holds.
 * - Delete asks first, and deletes its conversations and projects with it;
 *   the last workspace's is disabled.
 */
import { computed, nextTick, ref, watch } from 'vue'
import { ChevronDown, Plus, Trash2 } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Button from '../ui/Button.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import InlineError from '../ui/InlineError.vue'
import Tooltip from '../ui/Tooltip.vue'
import TruncatedText from '../ui/TruncatedText.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { pressOnKey } from '../ui/button-events'
import type { Action } from '../ui/operations'
import { useListReorder } from '../ui/useListReorder'
import WorkspaceIconPicker from '../workspaces/WorkspaceIconPicker.vue'
import WorkspaceTile from '../workspaces/WorkspaceTile.vue'
import { t } from '../workspaces/locales/text'
import { workspaceName, type WorkspaceColor, type WorkspaceIcon, type WorkspaceView } from '../workspaces/workspaces'
import SettingsGroup from './SettingsGroup.vue'
import SettingsListItem from './SettingsListItem.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSplit from './SettingsSplit.vue'

/** A workspace with how much it holds. */
export interface SettingsWorkspace extends WorkspaceView {
  conversations: number
  projects: number
}

/** A change of a workspace: each field it holds replaces its value. */
export interface WorkspaceEdit {
  name?: string
  icon?: WorkspaceIcon
  color?: WorkspaceColor
}

const props = defineProps<{
  workspaces: readonly SettingsWorkspace[]
  /** The workspace whose name field takes the focus with its name selected: one just made. */
  naming?: string | null
  /** Makes a workspace, which the page then selects and names. */
  onCreate?: Action
  /** Deletes the workspace once the user confirmed it. */
  onRemove?: Action<[id: string]>
}>()

const emit = defineEmits<{
  change: [id: string, edit: WorkspaceEdit]
  reorder: [ids: string[]]
}>()

const selectedId = defineModel<string | null>('selectedId', { default: null })
const splitOpen = ref(false)
const selected = computed(
  () => props.workspaces.find((workspace) => workspace.id === selectedId.value) ?? props.workspaces[0] ?? null,
)
const nameProblem = ref<string | null>(null)
const deleting = ref<SettingsWorkspace | null>(null)

const list = ref<HTMLElement>()
const order = useListReorder(
  list,
  () => props.workspaces.map((workspace) => workspace.id),
  (ids) => emit('reorder', ids),
)

/** Deletes the workspace asked about; the dialog closes once it is gone, if it still asks about it. */
async function confirmDelete(): Promise<void> {
  const asked = deleting.value
  if (!asked) {
    return
  }
  await props.onRemove?.(asked.id)
  if (deleting.value === asked) {
    deleting.value = null
  }
}

function select(id: string): void {
  selectedId.value = id
  splitOpen.value = true
}

function move(event: KeyboardEvent, id: string): void {
  if (event.altKey) {
    order.keydown(event, id)
  }
}

function checkName(value: string): string | null {
  return value.trim() ? null : t('nameRequired')
}

/** What a workspace's deletion takes with it, by its counts. */
function goes(workspace: SettingsWorkspace): string[] {
  return [
    ...(workspace.conversations ? [t('conversations', { count: workspace.conversations }, workspace.conversations)] : []),
    ...(workspace.projects ? [t('projects', { count: workspace.projects }, workspace.projects)] : []),
  ]
}

function contents(workspace: SettingsWorkspace): string {
  return `${t('conversations', { count: workspace.conversations }, workspace.conversations)} · ${t('projects', { count: workspace.projects }, workspace.projects)}`
}

// A workspace just made shows selected, its name ready to type over.
const nameField = ref<HTMLElement>()
watch(
  () => props.naming,
  async (id) => {
    if (!id) {
      return
    }
    selectedId.value = id
    splitOpen.value = true
    await nextTick()
    const input = nameField.value?.querySelector('input')
    input?.focus()
    input?.select()
  },
  { immediate: true },
)
</script>

<template>
  <SettingsPage wide fill :title="t('title')" :description="t('description')">
    <SettingsSplit v-model:detail-open="splitOpen" :detail-title="selected ? workspaceName(selected) : undefined">
      <template #list>
        <!-- Making one belongs to the list, so its button sits in the list's caption. -->
        <div class="flex h-8 select-none items-center pl-1">
          <span class="text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">{{ t('list') }}</span>
          <Button size="xs" class="ml-auto" @click="onCreate?.()">
            <Plus :size="12" /> {{ t('newShort') }}
          </Button>
        </div>
        <div ref="list" class="contents">
          <SettingsListItem
            v-for="workspace in workspaces"
            :key="workspace.id"
            :data-reorder-id="workspace.id"
            :class="{ 'bg-surface-float shadow-md': order.dragging.value === workspace.id }"
            :label="workspaceName(workspace)"
            :selected="workspace.id === selected?.id"
            @select="select(workspace.id)"
            @pointerdown="order.start($event, workspace.id)"
            @keydown="move($event, workspace.id)"
          >
            <template #leading><WorkspaceTile :icon="workspace.icon" :color="workspace.color" :size="24" /></template>
          </SettingsListItem>
        </div>
      </template>

      <template #detail>
        <div v-if="selected" class="flex flex-col gap-6">
          <SettingsGroup>
            <template #header>
              <header class="select-none">
                <div class="flex min-h-8 items-center gap-x-1.5">
                  <h3 class="min-w-0 text-[20px] font-medium leading-7 text-fg-emphasis wide:text-[15px] wide:leading-normal">
                    <TruncatedText :text="workspaceName(selected)" />
                  </h3>
                  <Tooltip :content="workspaces.length > 1 ? t('delete') : t('lastStays')" class="ml-auto">
                    <IconButton
                      size="sm"
                      :icon="Trash2"
                      variant="danger"
                      :aria-label="t('delete')"
                      :disabled="workspaces.length <= 1"
                      @click="deleting = selected"
                    />
                  </Tooltip>
                </div>
              </header>
            </template>
            <SettingsRow :label="t('name')">
              <template v-if="nameProblem" #detail>
                <InlineError :message="nameProblem" />
              </template>
              <div ref="nameField" class="w-56 max-w-full">
                <CommitTextInput
                  :key="selected.id"
                  :model-value="workspaceName(selected)"
                  :aria-label="t('name')"
                  :validate="checkName"
                  class="w-full"
                  @problem="nameProblem = $event"
                  @commit="emit('change', selected.id, { name: $event.trim() })"
                />
              </div>
            </SettingsRow>
            <SettingsRow :label="t('icon')">
              <Dropdown :overlay-store="appOverlayStore" align="end">
                <template #trigger="{ isOpen }">
                  <span
                    role="button"
                    tabindex="0"
                    :aria-label="t('iconLabel')"
                    class="flex cursor-default items-center gap-1 rounded-md p-1 transition-colors duration-200 ease-out"
                    :class="isOpen ? 'bg-active' : 'hover:bg-hover'"
                    @keydown="pressOnKey"
                    @keyup="pressOnKey"
                  >
                    <WorkspaceTile :icon="selected.icon" :color="selected.color" :size="28" />
                    <ChevronDown :size="ICON_PX.in28" class="text-fg-muted" />
                  </span>
                </template>
                <template #content>
                  <WorkspaceIconPicker
                    :icon="selected.icon"
                    :color="selected.color"
                    @change="emit('change', selected.id, $event)"
                  />
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow :label="t('contents')">
              <span class="text-chrome text-fg-muted">{{ contents(selected) }}</span>
            </SettingsRow>
          </SettingsGroup>
        </div>
      </template>
    </SettingsSplit>
    <ConfirmDialog
      :is-open="deleting !== null"
      :overlay-store="appOverlayStore"
      :title="t('deleteTitle', { name: deleting ? workspaceName(deleting) : '' })"
      :action="t('deleteConfirm')"
      :goes="deleting ? goes(deleting) : []"
      :on-confirm="confirmDelete"
      @close="deleting = null"
    >
      <p>{{ t('deleteMessage') }}</p>
    </ConfirmDialog>
  </SettingsPage>
</template>
