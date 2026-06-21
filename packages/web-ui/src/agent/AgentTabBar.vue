<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { AddLine } from '@mingcute/vue/add'
import { reportError } from '@demi/web-ui/infra/errors'
import { useContextMenuOwner } from '@demi/web-ui/composables/useContextMenuOwner'
import { t } from '@demi/web-ui/infra/i18n'
import { appOverlayStore } from '@demi/web-ui/overlay/appOverlay'
import Popover from '@demi/web-ui/ui/Popover.vue'
import Menu from '@demi/web-ui/ui/Menu.vue'
import MenuItem from '@demi/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demi/web-ui/ui/MenuDivider.vue'
import Tooltip from '@demi/web-ui/ui/Tooltip.vue'
import { useAgentWorkspace } from './workspace'
import { conversationStatus } from './conversation-status'
import type { ConversationState } from './types'
import AgentTabItem from './AgentTabItem.vue'
import ConversationListDropdown from './ConversationListDropdown.vue'

const DRAG_THRESHOLD = 3

const props = defineProps<{
  tabs: ConversationState[]
  allConversations: ConversationState[]
  activeTabId: string | null
}>()

const workspace = useAgentWorkspace()

const localTabs = ref<ConversationState[]>([...props.tabs])
const isDragging = ref(false)
const containerRef = ref<HTMLElement>()
const dragIdx = ref(-1)
const dragDeltaX = ref(0)
const tabShifts = ref<number[]>([])
const closingTabIds = ref(new Set<string>())
const enteringTabIds = ref(new Set<string>())
const isSettling = ref(false)

const focusedTabId = ref<string | null>(null)
const pendingRenameTabId = ref<string | null>(null)
const renameValue = ref('')

let startX = 0
let tabRects: DOMRect[] = []
let hasDragged = false
let currentNewIdx = -1
let closeTimer: ReturnType<typeof setTimeout> | null = null
let settleTimer: ReturnType<typeof setTimeout> | null = null

const {
  isOpen: isContextMenuOpen,
  anchorX: contextMenuX,
  anchorY: contextMenuY,
  open: openContextMenu,
  close: closeContextMenu,
} = useContextMenuOwner(() => {
  focusedTabId.value = null
})

const focusedTabIndex = computed(() =>
  localTabs.value.findIndex((tab) => tab.id === (focusedTabId.value ?? props.activeTabId)),
)
const hasOtherTabs = computed(() => localTabs.value.length > 1)
const hasTabsToLeft = computed(() => focusedTabIndex.value > 0)
const hasTabsToRight = computed(() => focusedTabIndex.value < localTabs.value.length - 1)

watch(() => props.tabs, (newTabs, oldTabs) => {
  if (isDragging.value) return

  if (oldTabs) {
    const oldIds = new Set(oldTabs.map((tab) => tab.id))
    for (const tab of newTabs) {
      if (!oldIds.has(tab.id)) enteringTabIds.value.add(tab.id)
    }
  }

  if (closingTabIds.value.size > 0) {
    const newTabIds = new Set(newTabs.map((tab) => tab.id))
    closingTabIds.value = new Set([...closingTabIds.value].filter((id) => newTabIds.has(id)))
  }

  localTabs.value = [...newTabs]

  if (enteringTabIds.value.size > 0) {
    nextTick(() => {
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          enteringTabIds.value = new Set()
        })
      })
    })
  }
}, { deep: true, immediate: true })

watch(pendingRenameTabId, (id) => {
  if (!id) return
  const tab = localTabs.value.find((entry) => entry.id === id)
  if (!tab) return
  renameValue.value = tab.title
})

// ── Navigation ──

function navigateToConversation(conversationId: string) {
  workspace.setActive(conversationId)
}

// ── Tab operations ──

function handleCreateTab(afterConversationId?: string) {
  workspace.createConversation(afterConversationId ? { afterId: afterConversationId } : {})
}

function handleCloseTabs(conversationIds: string[]) {
  if (conversationIds.length === 0) return
  if (closingTabIds.value.size > 0) finishClose()

  closingTabIds.value = new Set(conversationIds)
  closeTimer = setTimeout(finishClose, 300)
}

function handleRenameTab(conversationId: string, newTitle: string) {
  workspace.renameConversation(conversationId, newTitle)
}

function handleOpenConversation(conversationId: string) {
  workspace.setActive(conversationId)
}

// ── Context menu ──

function handleTabContextMenu(event: MouseEvent, tabId: string) {
  if (isDragging.value || isSettling.value) return
  focusedTabId.value = tabId
  openContextMenu(event)
}

function handleCopyConversationId() {
  const id = focusedTabId.value ?? props.activeTabId
  if (id) void navigator.clipboard.writeText(id)
}

function handleNewTabToRight() {
  const tabId = focusedTabId.value ?? props.activeTabId
  if (tabId) handleCreateTab(tabId)
}

function handleRenameFocused() {
  pendingRenameTabId.value = focusedTabId.value ?? props.activeTabId
}

function handleCloseFocused() {
  const tabId = focusedTabId.value ?? props.activeTabId
  if (tabId) handleCloseTab(tabId)
}

function handleCloseOthers() {
  const tabId = focusedTabId.value ?? props.activeTabId
  handleCloseTabs(localTabs.value.filter((tab) => tab.id !== tabId).map((tab) => tab.id))
}

function handleCloseToLeft() {
  handleCloseTabs(localTabs.value.slice(0, focusedTabIndex.value).map((tab) => tab.id))
}

function handleCloseToRight() {
  handleCloseTabs(localTabs.value.slice(focusedTabIndex.value + 1).map((tab) => tab.id))
}

function handleCloseAll() {
  handleCloseTabs(localTabs.value.map((tab) => tab.id))
}

// ── Rename ──

function submitRename() {
  const id = pendingRenameTabId.value
  if (!id) return
  const tab = localTabs.value.find((entry) => entry.id === id)
  const trimmed = renameValue.value.trim()
  pendingRenameTabId.value = null
  if (trimmed && trimmed !== tab?.title) {
    handleRenameTab(id, trimmed)
  }
}

function cancelRename() {
  pendingRenameTabId.value = null
}

// ── Tab close animation ──

function handleCloseTab(tabId: string) {
  if (closingTabIds.value.size > 0) finishClose()

  closingTabIds.value = new Set([tabId])
  closeTimer = setTimeout(finishClose, 300)
}

function finishClose() {
  if (closeTimer) {
    clearTimeout(closeTimer)
    closeTimer = null
  }
  if (closingTabIds.value.size === 0) return

  const ids = [...closingTabIds.value]
  closingTabIds.value = new Set()

  const closedSet = new Set(ids)
  localTabs.value = localTabs.value.filter((tab) => !closedSet.has(tab.id))

  void closeTabs(ids)
}

async function closeTabs(conversationIds: readonly string[]) {
  try {
    for (const conversationId of conversationIds) {
      await workspace.closeConversation(conversationId)
    }
  } catch (error) {
    localTabs.value = [...props.tabs]
    reportError('Failed to close conversation tabs', error, { userVisible: true })
  }
}

function handleTransitionEnd(event: TransitionEvent, tabId: string) {
  if (!closingTabIds.value.has(tabId)) return
  if (event.propertyName !== 'max-width') return
  finishClose()
}

// ── Drag ──

function getTabShift(index: number): number {
  if (!isDragging.value && !isSettling.value) return 0
  return index === dragIdx.value ? dragDeltaX.value : (tabShifts.value[index] ?? 0)
}

function handlePointerDown(event: PointerEvent, index: number) {
  if (event.button !== 0) return
  if (closingTabIds.value.size > 0) return
  if (isSettling.value) return
  const target = event.currentTarget as HTMLElement
  target.setPointerCapture(event.pointerId)
  startX = event.clientX
  dragIdx.value = index
  hasDragged = false
  currentNewIdx = index
  const children = containerRef.value!.children
  tabRects = Array.from(children).map((el) => (el as HTMLElement).getBoundingClientRect())
}

function handlePointerMove(event: PointerEvent) {
  if (dragIdx.value < 0 || isSettling.value) return

  const deltaX = event.clientX - startX
  if (!hasDragged && Math.abs(deltaX) < DRAG_THRESHOLD) return

  if (!hasDragged) {
    hasDragged = true
    isDragging.value = true
    tabShifts.value = new Array(localTabs.value.length).fill(0)
  }

  dragDeltaX.value = deltaX

  const draggedRect = tabRects[dragIdx.value]!
  const draggedLeft = draggedRect.left + deltaX
  const draggedRight = draggedRect.right + deltaX
  const draggedWidth = draggedRect.width
  const shifts = new Array(localTabs.value.length).fill(0)
  let newIdx = dragIdx.value

  for (let i = 0; i < tabRects.length; i++) {
    if (i === dragIdx.value) continue
    const otherCenter = tabRects[i]!.left + tabRects[i]!.width / 2

    if (i > dragIdx.value && draggedRight > otherCenter) {
      shifts[i] = -draggedWidth
      newIdx = Math.max(newIdx, i)
    } else if (i < dragIdx.value && draggedLeft < otherCenter) {
      shifts[i] = draggedWidth
      newIdx = Math.min(newIdx, i)
    }
  }

  tabShifts.value = shifts
  currentNewIdx = newIdx
}

function handlePointerUp() {
  if (dragIdx.value < 0 || isSettling.value) return

  const idx = dragIdx.value

  if (!hasDragged) {
    dragIdx.value = -1
    navigateToConversation(localTabs.value[idx]!.id)
    return
  }

  isSettling.value = true

  let targetDeltaX = 0
  if (currentNewIdx !== idx) {
    if (currentNewIdx > idx) {
      targetDeltaX = tabRects[currentNewIdx]!.right - tabRects[idx]!.right
    } else {
      targetDeltaX = tabRects[currentNewIdx]!.left - tabRects[idx]!.left
    }
  }

  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      dragDeltaX.value = targetDeltaX
    })
  })

  settleTimer = setTimeout(finishSettle, 140)
}

function finishSettle() {
  if (settleTimer) {
    clearTimeout(settleTimer)
    settleTimer = null
  }

  const idx = dragIdx.value
  const newIdx = currentNewIdx

  isSettling.value = false
  isDragging.value = false
  dragIdx.value = -1
  dragDeltaX.value = 0
  tabShifts.value = []

  if (newIdx !== idx && idx >= 0) {
    const [tab] = localTabs.value.splice(idx, 1)
    localTabs.value.splice(newIdx, 0, tab!)
    workspace.reorderTabs(localTabs.value.map((entry) => entry.id))
  }
}
</script>

<template>
  <div class="flex h-11 shrink-0 items-center bg-surface-base px-2">
    <div ref="containerRef" class="titlebar-no-drag flex min-w-0 items-center gap-0.5 overflow-x-auto scrollbar-hidden">
      <AgentTabItem
        v-for="(tab, index) in localTabs"
        :key="tab.id"
        :tab="tab"
        :is-active="tab.id === activeTabId"
        :status="conversationStatus(tab)"
        :provider-icon-id="tab.model.providerType"
        :is-closing="closingTabIds.has(tab.id)"
        :is-entering="enteringTabIds.has(tab.id)"
        :is-dragging="isDragging"
        :is-drag-target="isDragging && index === dragIdx"
        :is-settling="isSettling"
        :shift="getTabShift(index)"
        :is-renaming="pendingRenameTabId === tab.id"
        :rename-value="renameValue"
        @pointerdown="handlePointerDown($event, index)"
        @pointermove="handlePointerMove"
        @pointerup="handlePointerUp"
        @lostpointercapture="handlePointerUp"
        @transitionend="handleTransitionEnd($event, tab.id)"
        @contextmenu="handleTabContextMenu($event, tab.id)"
        @close="handleCloseTab(tab.id)"
        @rename-submit="submitRename"
        @rename-cancel="cancelRename"
        @update:rename-value="renameValue = $event"
      />
    </div>
    <Tooltip
      content=""
      class="titlebar-no-drag ml-1 flex size-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-fg-faint transition-colors hover:bg-hover hover:text-fg-muted"
      @click="handleCreateTab()"
    >
      <AddLine :size="14" />
    </Tooltip>
    <span class="titlebar-no-drag ml-auto">
      <ConversationListDropdown
        :conversations="allConversations"
        :active-tab-id="activeTabId"
        @select="handleOpenConversation"
      />
    </span>
  </div>
  <Popover
    :overlay-store="appOverlayStore"
    :is-open="isContextMenuOpen"
    :anchor-x="contextMenuX"
    :anchor-y="contextMenuY"
    :offset="0"
    @close="closeContextMenu"
  >
    <Menu @click="closeContextMenu">
      <MenuItem :label="t('agent.tab.newTabRight')" @select="handleNewTabToRight" />
      <MenuDivider />
      <MenuItem :label="t('common.rename')" @select="handleRenameFocused" />
      <MenuDivider />
      <MenuItem :label="t('common.close')" @select="handleCloseFocused" />
      <MenuItem :label="t('agent.tab.closeOthers')" :disabled="!hasOtherTabs" @select="handleCloseOthers" />
      <MenuItem :label="t('agent.tab.closeToLeft')" :disabled="!hasTabsToLeft" @select="handleCloseToLeft" />
      <MenuItem :label="t('agent.tab.closeToRight')" :disabled="!hasTabsToRight" @select="handleCloseToRight" />
      <MenuItem :label="t('agent.tab.closeAll')" @select="handleCloseAll" />
      <MenuDivider />
      <MenuItem :label="t('agent.tab.copyConversationId')" @select="handleCopyConversationId" />
    </Menu>
  </Popover>
</template>
