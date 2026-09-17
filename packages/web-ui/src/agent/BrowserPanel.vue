<script setup lang="ts">
import { computed, ref } from 'vue'
import { ArrowLeft, ArrowRight, Globe, Plus, RotateCw, X } from '@lucide/vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { appOverlayStore } from '../overlay/appOverlay'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import { closeTabs, tabsToClose, type TabCloseScope } from './tab-close'
import type { BrowserWorkTab } from './work-panel'

/** Local browser chrome; Host pages and navigation are not connected yet. */
const props = defineProps<{ tab: BrowserWorkTab }>()
const emit = defineEmits<{ update: [tab: BrowserWorkTab] }>()
const active = computed(() => props.tab.pages.find((page) => page.id === props.tab.activeId))
const menuId = ref<string | null>(null)
const menu = useContextMenuOwner(() => {
  menuId.value = null
})

function select(id: string): void {
  emit('update', { ...props.tab, activeId: id })
}

function add(): void {
  const page = { id: crypto.randomUUID(), title: 'New tab', address: '' }
  emit('update', { ...props.tab, pages: [...props.tab.pages, page], activeId: page.id })
}

function close(ids: string[]): void {
  const next = closeTabs(props.tab.pages, props.tab.activeId, ids)
  emit('update', { ...props.tab, pages: next.tabs, activeId: next.activeId })
}

function openMenu(event: MouseEvent, id: string): void {
  menuId.value = id
  menu.open(event)
}

function closeScope(scope: TabCloseScope): void {
  if (menuId.value) {
    close(tabsToClose(props.tab.pages, menuId.value, scope))
  }
  menu.close()
}

function updateAddress(address: string): void {
  emit('update', {
    ...props.tab,
    pages: props.tab.pages.map((page) => page.id === props.tab.activeId ? { ...page, address } : page),
  })
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex h-9 shrink-0 items-center border-t border-line-subtle px-2">
      <TabStrip class="flex-1" surface="raised">
        <TabItem
          v-for="page in tab.pages"
          :key="page.id"
          :tab="page"
          :is-active="page.id === tab.activeId"
          tabindex="0"
          @pointerdown="select(page.id)"
          @keydown.enter="select(page.id)"
          @keydown.space.prevent="select(page.id)"
          @contextmenu="openMenu($event, page.id)"
          @close="close([page.id])"
        >
          <template #mark><Globe :size="ICON_PX.markIn28" /></template>
        </TabItem>
        <template #trailing>
          <IconButton class="ml-1" :icon="Plus" size="sm" variant="ghost" aria-label="New browser tab" @click="add" />
        </template>
      </TabStrip>
    </div>
    <div class="flex h-9 shrink-0 items-center gap-1 px-2">
      <IconButton :icon="ArrowLeft" variant="ghost" aria-label="Back" disabled disabled-reason="Browser navigation is not connected yet" />
      <IconButton :icon="ArrowRight" variant="ghost" aria-label="Forward" disabled disabled-reason="Browser navigation is not connected yet" />
      <IconButton :icon="RotateCw" variant="ghost" aria-label="Refresh" disabled disabled-reason="Browser navigation is not connected yet" />
      <TextInput
        class="min-w-0 flex-1"
        :model-value="active?.address ?? ''"
        :disabled="!active"
        placeholder="Enter address"
        aria-label="Browser address"
        @update:model-value="updateAddress"
      />
    </div>
    <div class="flex min-h-0 flex-1 items-center justify-center border-t border-line-subtle text-[13px] text-fg-faint">
      {{ active ? 'Browser is not connected yet.' : 'No browser tabs open' }}
    </div>
    <Popover
      :overlay-store="appOverlayStore"
      :is-open="menu.isOpen.value"
      :anchor-x="menu.anchorX.value"
      :anchor-y="menu.anchorY.value"
      :anchor-context-el="menu.anchorContextEl.value"
      :offset="0"
      @close="menu.close"
    >
      <Menu>
        <MenuItem :icon="X" label="Close" @select="closeScope('self')" />
        <MenuItem
          v-for="item in [{ scope: 'others', label: 'Close others' }, { scope: 'right', label: 'Close to the right' }, { scope: 'left', label: 'Close to the left' }] as const"
          :key="item.scope"
          :label="item.label"
          :disabled="!menuId || tabsToClose(tab.pages, menuId, item.scope).length === 0"
          @select="closeScope(item.scope)"
        />
      </Menu>
    </Popover>
  </div>
</template>
