<script setup lang="ts">
import { computed, ref } from 'vue'
import { Plus } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import AgentTabItem from '@demicodes/web-ui/agent/AgentTabItem.vue'
import ConversationListDropdown from '@demicodes/web-ui/agent/ConversationListDropdown.vue'
import type { ConversationState } from '@demicodes/web-ui/agent/types'
import type { ConversationStatus } from '@demicodes/web-ui/agent/conversation-status'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import Popover from '@demicodes/web-ui/ui/Popover.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'

interface GalleryTab {
  tab: ConversationState
  status: ConversationStatus
  providerIconId: string
}

function createTab(
  id: string,
  title: string,
  status: ConversationStatus,
  providerIconId: string,
  seen = status !== 'done',
): GalleryTab {
  return {
    status,
    providerIconId,
    tab: {
      id,
      cwd: '/Users/zan/Projects/demi',
      title,
      createdAt: new Date().toISOString(),
      blocks: [],
      phase: status === 'active' ? 'running' : 'idle',
      queue: [],
      pendingSteers: [],
      model: {
        providerId: providerIconId,
        modelId: 'demo-model',
        thinkingEffort: null,
        serviceTierId: null,
      },
      draft: null,
      isResultSeen: seen,
      hasContent: status !== 'active' && status !== 'idle',
      lastError: status === 'error' ? 'rate limited' : null,
    },
  }
}

const tabs = ref<GalleryTab[]>([
  createTab('tab-1', 'Login test', 'done', 'anthropic', true),
  createTab('tab-2', 'Queued follow-up', 'active', 'openai'),
  createTab('tab-3', 'Cookie header', 'error', 'codex'),
  createTab('tab-4', 'Idle draft', 'idle', 'anthropic'),
  createTab('tab-5', 'Aborted run', 'aborted', 'openai'),
  createTab('tab-6', 'Unread result', 'done', 'codex', false),
  createTab('tab-7', 'Auth helper', 'done', 'anthropic', true),
  createTab('tab-8', 'Session cookie', 'idle', 'openai'),
])

const activeTabId = ref('tab-1')
const renamingTabId = ref<string | null>(null)
const renameValue = ref('')
const nextTab = ref(4)

const contextOpen = ref(false)
const contextX = ref(0)
const contextY = ref(0)
const contextEl = ref<HTMLElement | null>(null)
const contextTabId = ref<string | null>(null)

const conversations = computed(() => tabs.value.map((entry) => ({
  id: entry.tab.id,
  title: entry.tab.title,
  createdAt: entry.tab.createdAt,
})))

function selectTab(id: string): void {
  activeTabId.value = id
}

function closeTab(id: string): void {
  const index = tabs.value.findIndex((entry) => entry.tab.id === id)
  if (index < 0 || tabs.value.length === 1) return
  tabs.value.splice(index, 1)
  if (activeTabId.value === id) {
    activeTabId.value = tabs.value[Math.max(0, index - 1)]!.tab.id
  }
  if (renamingTabId.value === id) {
    renamingTabId.value = null
  }
}

function addTab(): void {
  const id = `tab-${nextTab.value++}`
  tabs.value.push(createTab(id, 'Untitled', 'done', 'anthropic'))
  activeTabId.value = id
}

function beginRename(id: string): void {
  const entry = tabs.value.find((item) => item.tab.id === id)
  if (!entry) return
  renamingTabId.value = id
  renameValue.value = entry.tab.title
}

function submitRename(): void {
  const entry = tabs.value.find((item) => item.tab.id === renamingTabId.value)
  if (entry && renameValue.value.trim()) {
    entry.tab.title = renameValue.value.trim()
  }
  renamingTabId.value = null
}

function openContextMenu(event: MouseEvent, id: string): void {
  selectTab(id)
  contextTabId.value = id
  contextX.value = event.clientX
  contextY.value = event.clientY
  contextEl.value = event.currentTarget instanceof HTMLElement ? event.currentTarget : null
  contextOpen.value = true
}

function closeContextMenu(): void {
  contextOpen.value = false
}
</script>

<template>
  <div class="flex h-11 items-center">
    <div class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto">
      <AgentTabItem
        v-for="entry in tabs"
        :key="entry.tab.id"
        :tab="entry.tab"
        :is-active="entry.tab.id === activeTabId"
        :status="entry.status"
        :is-closing="false"
        :is-entering="false"
        :is-dragging="false"
        :is-drag-target="false"
        :is-settling="false"
        :shift="0"
        :is-renaming="renamingTabId === entry.tab.id"
        :rename-value="renameValue"
        :provider-icon-id="entry.providerIconId"
        @pointerdown="selectTab(entry.tab.id)"
        @contextmenu="openContextMenu($event, entry.tab.id)"
        @close="closeTab(entry.tab.id)"
        @rename-submit="submitRename"
        @rename-cancel="renamingTabId = null"
        @update:rename-value="renameValue = $event"
      />
    </div>
    <Tooltip
      content="New tab"
      class="ml-1 flex size-6 shrink-0 cursor-default items-center justify-center rounded-md text-fg-faint transition-colors hover:bg-hover hover:text-fg-muted"
      @click="addTab"
    >
      <Plus :size="14" />
    </Tooltip>
    <span class="ml-auto">
      <ConversationListDropdown
        :conversations="conversations"
        :active-tab-id="activeTabId"
        @select="selectTab"
      />
    </span>
  </div>

  <Popover
    :overlay-store="appOverlayStore"
    :is-open="contextOpen"
    :anchor-x="contextX"
    :anchor-y="contextY"
    :anchor-context-el="contextEl"
    :offset="0"
    @close="closeContextMenu"
  >
    <Menu @click="closeContextMenu">
      <MenuItem label="Rename" @select="contextTabId && beginRename(contextTabId)" />
      <MenuItem label="New tab" @select="addTab" />
      <MenuDivider />
      <MenuItem label="Close" :disabled="tabs.length === 1" @select="contextTabId && closeTab(contextTabId)" />
    </Menu>
  </Popover>
</template>
