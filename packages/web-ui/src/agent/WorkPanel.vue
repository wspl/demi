<script setup lang="ts">
import { computed, ref, shallowRef, watch } from 'vue'
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
import { keptContents, pinnedData, shownSelection, type PanelState, type PinnedTabs } from './panel-tabs'
import { resolvePanelTab, type PanelTabKind } from './panel-kinds/kind'

/**
 * The work panel's frame (`web-application.md` § Work panel): the pinned
 * tab of each pinned kind, the strip of the user's tabs, and what the
 * selected one shows. Every tab is of a kind a plugin registers; the panel
 * shows it through its kind and knows nothing else about it. A content once
 * shown stays on the page, hidden and told it is not shown, until its tab
 * closes or the panel binds other kinds, as for another conversation.
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
const shownTab = computed(() => tabs.value.find((item) => item.tab.id === selection.value) ?? null)
/** The ids of what the panel has: its pinned kinds and its tabs. */
const present = computed(() => [...pinnedTabs.value.map((item) => item.kind.kind), ...props.panel.tabs.map((tab) => tab.id)])
/** Every content the panel keeps, by the id of its tab or pinned kind. */
const kept = shallowRef<readonly string[]>(keptContents([], selection.value, present.value))
watch([selection, present], ([shown, ids]) => {
  kept.value = keptContents(kept.value, shown, ids)
})
// Kinds bound anew belong to another conversation, or another set of pages: nothing of the old ones stays.
watch(() => props.kinds, () => {
  kept.value = keptContents([], selection.value, present.value)
})
/** The kept contents, each with its kind and data. */
const contents = computed(() => kept.value.flatMap((id) => {
  const pinned = pinnedTabs.value.find((item) => item.kind.kind === id)
  if (pinned) {
    return [{ id, kind: pinned.kind, data: pinned.data, pinned: true }]
  }
  const tab = tabs.value.find((item) => item.tab.id === id)
  return tab?.kind ? [{ id, kind: tab.kind, data: tab.data, pinned: false }] : []
}))
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
      <!-- The pinned tabs give way before the strip: in a narrow panel their titles truncate, down to their
           marks and badges, so the strip keeps room for its selected tab whole, up to a tab's full 160px
           and the New tab control beside it. A pinned tab is a grid so that its smallest width is its mark
           and badge, never a cut badge; only then does the strip give way, its selected tab's title
           truncating. -->
      <div v-if="pinnedTabs.length" class="flex shrink-[100] items-center gap-1" role="group" aria-label="Pinned tabs">
        <button
          v-for="item in pinnedTabs"
          :key="item.kind.kind"
          type="button"
          :aria-pressed="item.kind.kind === selection"
          :title="item.kind.title(item.data)"
          class="grid h-7 items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-chrome hover:bg-surface-base hover:text-fg"
          :class="[
            item.kind.badge ? 'grid-cols-[auto_minmax(0,auto)_auto]' : 'grid-cols-[auto_minmax(0,auto)]',
            item.kind.kind === selection ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle',
          ]"
          @click="pick(item.kind, item.data)"
        >
          <component :is="item.kind.mark" :data="item.data" />
          <span class="truncate">{{ item.kind.title(item.data) }}</span>
          <component :is="item.kind.badge" v-if="item.kind.badge" :data="item.data" />
        </button>
      </div>
      <TabStrip class="min-w-0 grow" :class="tabs.length > 0 ? 'basis-48' : 'basis-0'" surface="raised">
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
      <!-- Each content keeps what it shows while another tab is selected; only the shown one is told it is. -->
      <div
        v-for="content in contents"
        v-show="content.id === selection"
        :key="content.id"
        class="flex min-h-0 flex-1 flex-col"
      >
        <component
          :is="content.kind.content"
          :tab-id="content.id"
          :data="content.data"
          :shown="content.id === selection"
          @update="content.pinned ? emit('updatePinned', content.id, $event) : emit('updateTab', content.id, $event)"
          @close="content.pinned || emit('closeTabs', [content.id])"
        />
      </div>
      <template v-if="!contents.some((content) => content.id === selection)">
      <div
        v-if="shownTab"
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
      </template>
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
