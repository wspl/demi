<script setup lang="ts">
import { computed, ref } from 'vue'
import { PanelRightClose, Plus, X } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { appOverlayStore } from '../overlay/appOverlay'
import type { TitleText } from '../ui/ui-text'
import { tabsToClose, type TabCloseScope } from './tab-close'
import { pinnedData, shownSelection, type PanelState, type PinnedTabs } from './panel-tabs'
import { resolvePanelTab, type PanelTabKind } from './panel-kinds/kind'

/**
 * The work panel's frame (`web-application.md` § Work panel): the pinned
 * tab of each pinned kind, the strip of the user's tabs, and what the
 * selected one shows. Every tab is of a kind a plugin registers; the panel
 * shows it through its kind and knows nothing else about it.
 */
const props = defineProps<{
  /** The selection and the user's tabs. */
  panel: PanelState
  /** The pinned tabs' data, by kind. */
  pinned: PinnedTabs
  kinds: readonly PanelTabKind[]
  /**
   * The conversation's first send has not created it yet, so it has no
   * working directory: the body says when files and changes appear.
   */
  beforeFirstMessage?: boolean
}>()
const emit = defineEmits<{
  select: [selection: string]
  addTab: [kind: string, data: unknown]
  updateTab: [id: string, data: unknown]
  updatePinned: [kind: string, data: unknown]
  closeTabs: [ids: string[]]
  close: []
}>()

/** Each pinned kind with the data its tab shows. */
const pinnedTabs = computed(() =>
  props.kinds
    .filter((kind) => kind.pinned)
    .map((kind) => ({ kind, data: pinnedData(props.pinned, kind) })),
)
const selection = computed(() => shownSelection(props.panel, props.kinds))

/** Picking a pinned tab may change what it shows, even while it is selected. */
function pick(kind: PanelTabKind, data: unknown): void {
  if (kind.picked) {
    emit('updatePinned', kind.kind, kind.picked(data))
  }
  emit('select', kind.kind)
}

/** Each tab with its kind and checked data, in the user's order. */
const tabs = computed(() => props.panel.tabs.map((tab) => resolvePanelTab(tab, props.kinds)))
const shownPinned = computed(() => pinnedTabs.value.find((item) => item.kind.kind === selection.value) ?? null)
const shownTab = computed(() => tabs.value.find((item) => item.tab.id === selection.value) ?? null)
/** The kinds the strip's new-tab control offers. */
const creatable = computed(() => props.kinds.filter((kind) => kind.create))

/** Why the strip cannot make a tab of `kind` now, or null when it can. */
function unavailable(kind: PanelTabKind): string | null {
  return kind.create?.unavailable?.() ?? null
}
const menuId = ref<string | null>(null)
const menu = useContextMenuOwner(() => {
  menuId.value = null
})

function openMenu(event: MouseEvent, id: string): void {
  menuId.value = id
  menu.open(event)
}

/** The tab menu's commands that close several tabs at once. */
const closeManyItems: readonly { scope: Exclude<TabCloseScope, 'self'>; label: TitleText }[] = [
  { scope: 'others', label: 'Close Others' },
  { scope: 'right', label: 'Close to the Right' },
  { scope: 'left', label: 'Close to the Left' },
]

function closeScope(scope: TabCloseScope): void {
  if (menuId.value) {
    emit('closeTabs', tabsToClose(props.panel.tabs, menuId.value, scope))
  }
  menu.close()
}
</script>

<template>
  <aside class="flex h-full min-w-0 flex-col overflow-hidden border-l border-line bg-surface text-fg">
    <div class="flex h-11 shrink-0 items-center gap-1 pl-2 pr-3">
      <div v-if="pinnedTabs.length" class="flex shrink-0 items-center gap-1" role="group" aria-label="Pinned tabs">
        <button
          v-for="item in pinnedTabs"
          :key="item.kind.kind"
          type="button"
          :aria-pressed="item.kind.kind === selection"
          :title="item.kind.title(item.data)"
          class="flex h-7 shrink-0 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-chrome hover:bg-surface-base hover:text-fg"
          :class="item.kind.kind === selection ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle'"
          @click="pick(item.kind, item.data)"
        >
          <component :is="item.kind.mark" :data="item.data" />
          <span>{{ item.kind.title(item.data) }}</span>
          <component :is="item.kind.badge" v-if="item.kind.badge" :data="item.data" />
        </button>
      </div>
      <TabStrip class="min-w-0 flex-1" surface="raised">
        <TabItem
          v-for="item in tabs"
          :key="item.tab.id"
          :title="item.title"
          :is-active="item.tab.id === selection"
          tabindex="0"
          @pointerdown="emit('select', item.tab.id)"
          @keydown.enter="emit('select', item.tab.id)"
          @keydown.space.prevent="emit('select', item.tab.id)"
          @contextmenu="openMenu($event, item.tab.id)"
          @close="emit('closeTabs', [item.tab.id])"
        >
          <template v-if="item.kind" #mark><component :is="item.kind.mark" :data="item.data" /></template>
        </TabItem>
        <!-- A plain plus says enough beside tabs of the one kind it makes; otherwise each kind shows its own icon. -->
        <template v-if="creatable.length > 0" #trailing>
          <!-- One tip: why the kind cannot be made now, else what the control makes. -->
          <Tooltip
            v-for="kind in creatable"
            :key="kind.kind"
            :content="unavailable(kind) ?? kind.create!.label"
            class="ml-1 shrink-0"
          >
            <IconButton
              :icon="tabs.length > 0 && creatable.length === 1 ? Plus : kind.create!.icon"
              size="sm"
              variant="ghost"
              :aria-label="kind.create!.label"
              :disabled="unavailable(kind) !== null"
              @click="emit('addTab', kind.kind, kind.create!.data())"
            />
          </Tooltip>
        </template>
      </TabStrip>
      <Tooltip content="Close panel" class="shrink-0">
        <IconButton :icon="PanelRightClose" variant="ghost" aria-label="Close panel" @click="emit('close')" />
      </Tooltip>
    </div>
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!-- A host that shows only the strip, as a specimen of it does, fills the body itself. -->
      <slot>
      <component
        :is="shownPinned.kind.content"
        v-if="shownPinned"
        :key="shownPinned.kind.kind"
        :tab-id="shownPinned.kind.kind"
        :data="shownPinned.data"
        shown
        @update="emit('updatePinned', shownPinned.kind.kind, $event)"
      />
      <component
        :is="shownTab.kind.content"
        v-else-if="shownTab?.kind"
        :key="shownTab.tab.id"
        :tab-id="shownTab.tab.id"
        :data="shownTab.data"
        shown
        @update="emit('updateTab', shownTab.tab.id, $event)"
        @close="emit('closeTabs', [shownTab.tab.id])"
      />
      <div
        v-else-if="shownTab"
        class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-6 text-center text-[13px] text-fg-faint"
      >
        <span>This tab cannot be shown.</span>
        <span class="font-mono text-[11px]">{{ shownTab.tab.kind }}</span>
      </div>
      <div
        v-else
        class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-6 text-center text-[13px] text-fg-faint"
      >
        <span>{{ beforeFirstMessage ? 'Files and changes appear after the first message.' : 'Nothing open' }}</span>
      </div>
      </slot>
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
          v-for="item in closeManyItems"
          :key="item.scope"
          :label="item.label"
          :disabled="!menuId || tabsToClose(panel.tabs, menuId, item.scope).length === 0"
          @select="closeScope(item.scope)"
        />
      </Menu>
    </Popover>
  </aside>
</template>
