<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { computed, h, nextTick, ref, shallowRef, watch, type Component, type FunctionalComponent } from 'vue'
import { ArrowLeftToLine, ArrowRightToLine, ChevronDown, ChevronLeft, CopyX, PanelRightClose, X } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import Tooltip from '../ui/Tooltip.vue'
import TabItem from './TabItem.vue'
import TabStrip from './TabStrip.vue'
import { vContextMenu, type ContextMenuEntry, type ContextMenuGroup } from '../ui/context-menu'
import { ICON_PX } from '../ui/icon-metrics'
import type { MenuListItem } from '../ui/menu-context'
import { usePhone } from '../ui/phone'
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
 *
 * On a phone it takes the whole screen (SidebarLayout) under one bar of its
 * own: a back button, its chevron alone and named Back to Conversation,
 * which goes back to the conversation as it was, as a swipe from the
 * screen's left edge does (SidebarLayout); the strip; and the strip's
 * controls. The desktop's close control is that back.
 */
const props = withDefaults(defineProps<{
  /** The selection and the user's tabs. */
  panel: PanelState
  /**
   * Whether the owner has read the tabs, true unless it says not: the tabs
   * a first read brings were there before the panel showed them, so they
   * come with no motion; a tab opened or closed after it moves.
   */
  tabsRead?: boolean
  /** The pinned tabs' data, by kind. */
  pinned: PinnedTabs
  kinds: readonly PanelTabKind[]
  /**
   * The conversation's first send has not created it yet, so it has no
   * working directory: the body says when files and changes appear.
   */
  beforeFirstMessage?: boolean
}>(), {
  tabsRead: true,
})
const emit = defineEmits<{
  select: [selection: string]
  /** The tab `id` moved to `index` among the others, as the user dragged it. */
  moveTab: [id: string, index: number]
  updateTab: [id: string, data: unknown]
  updatePinned: [kind: string, data: unknown]
  closeTabs: [ids: string[]]
  close: []
}>()

/**
 * Each pinned kind with the data its tab shows and its own short title, the
 * title of the data it starts from, such as File's before a file is open, so
 * the strip's label never follows what the tab shows.
 */
const pinnedTabs = computed(() =>
  props.kinds.flatMap((kind) => kind.pinned
    ? [{ kind, data: pinnedData(props.pinned, kind), label: kind.title(kind.pinned.data()) }]
    : []),
)
const selection = computed(() => shownSelection(props.panel, props.kinds))
const phone = usePhone()

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
/**
 * The strip shows tabs nobody opened or closed just now: another
 * conversation's, or another set of pages', in place of its own, or the
 * ones the first read of the tabs brings. They come and go at once, with no
 * motion (`TabStrip`'s `still`), for the render that shows them.
 */
const replacing = ref(false)
function replaceTabs(): void {
  replacing.value = true
  void nextTick(() => {
    replacing.value = false
  })
}
// Kinds bound anew belong to another conversation, or another set of pages: nothing of the old ones stays.
watch(() => props.kinds, () => {
  kept.value = keptContents([], selection.value, present.value)
  replaceTabs()
})
watch(() => props.tabsRead, (read) => {
  if (read) {
    replaceTabs()
  }
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
/** The tab menu's commands that close several tabs at once. */
const closeManyItems = computed<readonly { scope: Exclude<TabCloseScope, 'self'>; label: TitleText; icon: Component }[]>(() => [
  { scope: 'others', label: t('panel.closeOthers'), icon: CopyX },
  { scope: 'right', label: t('panel.closeRight'), icon: ArrowRightToLine },
  { scope: 'left', label: t('panel.closeLeft'), icon: ArrowLeftToLine },
])

function closeScope(id: string, scope: TabCloseScope): void {
  emit('closeTabs', tabsToClose(props.panel.tabs, id, scope))
}

/** A tab's mark as the strip draws it, as an icon of the list of every tab: its spinner, its picture or its kind's mark. */
function listMark(mark: Component | null, data: unknown, picture: string | null, busy: boolean): FunctionalComponent {
  return () => busy
    ? h(IndeterminateSpinner, { size: ICON_PX.markIn28 })
    : picture
      ? h('img', { src: picture, alt: '', width: ICON_PX.markIn28, height: ICON_PX.markIn28, draggable: false })
      : mark ? h(mark, { data }) : null
}

/** Every tab, the pinned ones first, by its mark and title, as the strip's list shows them. */
const listedTabs = computed<MenuListItem[]>(() => [
  ...pinnedTabs.value.map((item) => ({
    id: item.kind.kind,
    label: item.kind.title(item.data),
    icon: listMark(item.kind.mark, item.data, null, false),
  })),
  ...tabs.value.map((item) => ({
    id: item.tab.id,
    label: item.title,
    icon: listMark(
      item.kind?.mark ?? null,
      item.data,
      item.kind?.icon?.(item.data, item.tab.id) ?? null,
      item.kind?.busy?.(item.data, item.tab.id) ?? false,
    ),
  })),
])

/**
 * A tab's menu: what its kind offers, then the Close commands, as a web
 * browser's tab menu.
 */
function tabMenu(id: string): () => ContextMenuGroup[] {
  return () => {
    const item = tabs.value.find((each) => each.tab.id === id)
    const own: ContextMenuEntry[] = (item ? item.kind?.commands?.(item.data, item.tab.id) ?? [] : [])
      .map((command) => ({ icon: command.icon, label: command.label, disabled: command.disabled, run: () => command.run() }))
    return [
      own,
      [
        { icon: X, label: shared('common.close'), run: () => closeScope(id, 'self') },
        ...closeManyItems.value.map((close) => ({
          icon: close.icon,
          label: close.label,
          disabled: tabsToClose(props.panel.tabs, id, close.scope).length === 0,
          run: () => closeScope(id, close.scope),
        })),
      ],
    ]
  }
}

/** A tab dragged along the strip, by its place in it. */
function reorder(from: number, to: number): void {
  const moved = props.panel.tabs[from]
  if (moved) {
    emit('moveTab', moved.id, to)
  }
}
</script>

<template>
  <aside
    class="flex h-full min-w-0 flex-col overflow-hidden bg-surface text-fg"
    :class="phone ? 'pb-[var(--safe-bottom)]' : 'border-l border-line'"
  >
    <div
      class="flex shrink-0 items-center gap-1"
      :class="phone ? 'h-[calc(2.75rem+var(--safe-top))] pl-1 pr-1.5 pt-[var(--safe-top)]' : 'h-11 pl-2 pr-3'"
    >
      <!-- The chevron alone, as a phone's own back button with no room for a title; its name says where it goes. -->
      <IconButton
        v-if="phone"
        :icon="ChevronLeft"
        variant="ghost"
        :aria-label="t('session.backToConversation')"
        @click="emit('close')"
      />
      <TabStrip
        class="min-w-0 grow"
        surface="raised"
        :still="replacing"
        @reorder="reorder"
      >
        <!-- The pinned tabs lead the strip in one shape whatever is selected: the mark, the kind's own
             short title and the badge, such as Change's counts, with what the tab shows, such as the
             open file's name, in the tooltip, so selecting one or opening a file widens nothing. Each
             is a tab as the strip's own are: a press selects it, as do Enter and Space, and the arrows reach it. -->
        <template v-if="pinnedTabs.length" #leading>
          <Tooltip
            v-for="item in pinnedTabs"
            :key="item.kind.kind"
            tag="span"
            :content="item.kind.title(item.data)"
            placement="bottom"
            role="tab"
            tabindex="0"
            :aria-selected="item.kind.kind === selection"
            class="flex h-tab items-center gap-1.5 whitespace-nowrap rounded-md px-1.5 text-chrome select-none hover:bg-surface-base hover:text-fg"
            :class="item.kind.kind === selection ? 'bg-surface-base text-fg-emphasis' : 'text-fg-subtle'"
            @pointerdown.left="emit('select', item.kind.kind)"
            @keydown.enter.self="emit('select', item.kind.kind)"
            @keydown.space.self.prevent="emit('select', item.kind.kind)"
          >
            <component :is="item.kind.mark" :data="item.data" />
            <!-- The badge's smaller figures stand on the title's baseline, not centered beside it. -->
            <span class="flex items-baseline gap-1.5">
              <span>{{ item.label }}</span>
              <component :is="item.kind.badge" v-if="item.kind.badge" :data="item.data" />
            </span>
          </Tooltip>
        </template>
        <TabItem
          v-for="item in tabs"
          :key="item.tab.id"
          :title="item.title"
          :is-active="item.tab.id === selection"
          :busy="item.kind?.busy?.(item.data, item.tab.id) ?? false"
          :icon="item.kind?.icon?.(item.data, item.tab.id) ?? null"
          @select="emit('select', item.tab.id)"
          v-context-menu="tabMenu(item.tab.id)"
          @close="emit('closeTabs', [item.tab.id])"
        >
          <template v-if="item.kind" #mark><component :is="item.kind.mark" :data="item.data" /></template>
        </TabItem>
        <!-- With more tabs than fit, a list of every tab, as VS Code's Show Opened Editors: picking one selects it. -->
        <template #overflow>
          <Dropdown :overlay-store="appOverlayStore">
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('panel.allTabs')">
                <IconButton
                  :icon="ChevronDown"
                  variant="ghost"
                  :aria-label="t('panel.allTabs')"
                  :pressed="isOpen"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu
                :items="listedTabs"
                :selected-id="selection ?? undefined"
                @select="(id) => { emit('select', id); close() }"
              />
            </template>
          </Dropdown>
        </template>
      </TabStrip>
      <Tooltip v-if="!phone" :content="t('session.closePanel')" class="shrink-0">
        <IconButton :icon="PanelRightClose" variant="ghost" :aria-label="t('session.closePanel')" @click="emit('close')" />
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
        <span>{{ t('panel.cannotShow') }}</span>
        <span class="font-mono text-[11px]">{{ shownTab.tab.kind }}</span>
      </div>
      <div
        v-else
        class="flex flex-1 select-none flex-col items-center justify-center gap-1 px-6 text-center text-[13px] text-fg-faint"
      >
        <span>{{ beforeFirstMessage ? t('panel.noFiles') : t('panel.nothingOpen') }}</span>
      </div>
      </template>
      </slot>
    </div>
  </aside>
</template>
