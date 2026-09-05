<script setup lang="ts">
import { computed, ref } from 'vue'
import { Plus } from '@lucide/vue'
import AgentTabItem from '@demicodes/web-ui/agent/AgentTabItem.vue'
import ConversationListDropdown from '@demicodes/web-ui/agent/ConversationListDropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import Popover from '@demicodes/web-ui/ui/Popover.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { useContextMenuOwner } from '@demicodes/web-ui/composables/useContextMenuOwner'
import { useConversations } from './store'

const props = defineProps<{ activeId: string | null }>()
const emit = defineEmits<{ open: [id: string]; close: [id: string]; create: [] }>()
const store = useConversations()
const tabs = computed(() => store.tabs.flatMap((id) => store.items.find((c) => c.id === id) ?? []))
const summaries = computed(() =>
  store.items
    .filter((c) => !c.archived)
    .map((c) => ({ id: c.id, title: c.title, createdAt: c.updatedAt })),
)
const menu = useContextMenuOwner()
const menuId = ref<string | null>(null)
const renaming = ref<string | null>(null)
const renameValue = ref('')

function context(event: MouseEvent, id: string) {
  menuId.value = id
  menu.open(event)
}
function rename() {
  const item = store.items.find((c) => c.id === menuId.value)
  if (!item) return
  renaming.value = item.id
  renameValue.value = item.title
  menu.close()
}
function submitRename() {
  if (renaming.value) store.rename(renaming.value, renameValue.value)
  renaming.value = null
}
function closeFromMenu() {
  if (menuId.value) emit('close', menuId.value)
  menu.close()
}
</script>

<template>
  <nav class="flex h-11 shrink-0 items-center px-2" aria-label="Open conversations">
    <div class="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto" role="tablist">
      <AgentTabItem
        v-for="tab in tabs"
        :key="tab.id"
        :tab="tab"
        :is-active="tab.id === props.activeId"
        :status="tab.status"
        :is-closing="false"
        :is-entering="false"
        :is-dragging="false"
        :is-drag-target="false"
        :is-settling="false"
        :shift="0"
        :is-renaming="renaming === tab.id"
        :rename-value="renameValue"
        :provider-icon-id="null"
        @pointerdown="emit('open', tab.id)"
        @contextmenu="context($event, tab.id)"
        @close="emit('close', tab.id)"
        @rename-submit="submitRename"
        @rename-cancel="renaming = null"
        @update:rename-value="renameValue = $event"
      />
    </div>
    <Tooltip content="New conversation">
      <IconButton
        :icon="Plus"
        variant="ghost"
        size="sm"
        aria-label="New conversation"
        @click="emit('create')"
      />
    </Tooltip>
    <ConversationListDropdown
      :conversations="summaries"
      :active-tab-id="activeId"
      @select="emit('open', $event)"
    />
  </nav>
  <Popover
    :overlay-store="appOverlayStore"
    :is-open="menu.isOpen.value"
    :anchor-x="menu.anchorX.value"
    :anchor-y="menu.anchorY.value"
    :anchor-context-el="menu.anchorContextEl.value"
    :offset="0"
    @close="menu.close()"
  >
    <Menu>
      <MenuItem label="Rename" @select="rename" />
      <MenuItem label="Close" @select="closeFromMenu" />
    </Menu>
  </Popover>
</template>
