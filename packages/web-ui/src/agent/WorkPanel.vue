<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { computed, h, nextTick, provide, ref, shallowRef, watch, type Component, type FunctionalComponent } from 'vue'
import { ArrowLeftToLine, ArrowRightToLine, ChevronDown, ChevronLeft, CopyX, ListX, PanelRightClose, X } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import Tooltip from '../ui/Tooltip.vue'
import HostOfflinePage from './HostOfflinePage.vue'
import PaneScope from './PaneScope.vue'
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
import { sheetDismissKey } from '../files/panel-sheet'
import SheetCloseButton from '../files/SheetCloseButton.vue'
import type { TabUpdate } from '../plugins/page'

/**
 * The work panel's frame: the pinned tab of each pinned kind, the strip of the
 * user's tabs, and what the selected one shows.
 *
 * Every tab is of a kind a plugin registers. The panel shows it through its
 * kind and knows nothing else about it. A content once shown stays on the page,
 * hidden and told it is not shown, until its tab closes or the panel binds
 * other kinds, as for another conversation.
 *
 * On a phone, the panel takes the whole screen (SidebarLayout) under one bar of
 * its own, with three parts:
 *
 * - a back button, its chevron alone and named Back to Conversation, which goes
 *   back to the conversation as it was, as a swipe from the screen's left edge
 *   does (SidebarLayout)
 * - the strip
 * - the strip's controls
 *
 * The desktop's close control is that back.
 *
 * A view the conversation opens on a phone, such as a picture tapped, shows as
 * an overlay instead (`sheet`): the selected view alone, with no strip and no
 * bar of the panel's, its own bar ending with Close, which closes the overlay
 * to the conversation, as the panel's back does (`SheetCloseButton`).
 */
const props = withDefaults(defineProps<{
  /** The selection and the user's tabs. */
  panel: PanelState
  /**
   * Whether the owner has read the tabs, true unless it says not. The tabs a
   * first read brings were there before the panel showed them, so they come
   * with no motion. A tab opened or closed after it moves.
   */
  tabsRead?: boolean
  /** The pinned tabs' data, by kind. */
  pinned: PinnedTabs
  kinds: readonly PanelTabKind[]
  /**
   * The conversation's first send has not created it yet, so it has no working
   * directory. The body says when files and changes appear.
   */
  beforeFirstMessage?: boolean
  /**
   * The name of the conversation's primary Host while its runner is not
   * connected; the body then shows that, in place of the tabs' contents, which
   * stay mounted behind it.
   */
  hostOffline?: string | null
  /**
   * Whether a phone draws the panel as an overlay over the conversation: only
   * the selected view, its own bar ending with Close, with no tab strip. The tabs button
   * opens the full panel instead, where this is false (`work.ts`, `ENTRY_SHEET`).
   */
  sheet?: boolean
}>(), {
  hostOffline: null,
  tabsRead: true,
  sheet: false,
})
const emit = defineEmits<{
  select: [selection: string]
  /** The tab `id` moved to `index` among the others, as the user dragged it. */
  moveTab: [id: string, index: number]
  /** A tab's next data; `how` says when it is no step of the history (`TabUpdate`). */
  updateTab: [id: string, data: unknown, how?: TabUpdate]
  updatePinned: [kind: string, data: unknown, how?: TabUpdate]
  closeTabs: [ids: string[]]
  close: []
}>()

/**
 * Each pinned kind with the data its tab shows and its own short title. The
 * title is that of the data the tab starts from, such as File's before a file
 * is open, so the strip's label never follows what the tab shows.
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
/**
 * The contents the panel showed first, as the page loaded, the panel opened or
 * its kinds were bound anew: each says what it reads from its start
 * (`PaneScope`'s `first`). A content shown by a switch is none of them.
 */
const firstShown = shallowRef<ReadonlySet<string>>(new Set(kept.value))
/**
 * A number for each kind as bound, which a pane's key holds: a kind bound anew (for another
 * conversation, or its plugin turned off and on) is a pane of its own, with its own first view,
 * while turning an unrelated plugin on or off leaves the shown pane mounted.
 */
const bindings = new WeakMap<object, number>()
let bound = 0
function bindingOf(kind: object): number {
  let number = bindings.get(kind)
  if (number === undefined) {
    bound += 1
    number = bound
    bindings.set(kind, number)
  }
  return number
}
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
// Kinds bound anew belong to another conversation, or another set of pages:
// nothing of the old ones stays.
watch(() => props.kinds, () => {
  kept.value = keptContents([], selection.value, present.value)
  firstShown.value = new Set(kept.value)
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
    return [{ id, kind: pinned.kind, data: pinned.data, pinned: true, name: pinned.kind.subject?.(pinned.data) ?? pinned.kind.title(pinned.data) }]
  }
  const tab = tabs.value.find((item) => item.tab.id === id)
  return tab?.kind ? [{ id, kind: tab.kind, data: tab.data, pinned: false, name: tab.kind.subject?.(tab.data) ?? tab.title }] : []
}))
/** Whether the panel is a phone's overlay (`sheet`), off a phone never, whatever the owner says. */
const overlay = computed(() => phone.value && props.sheet)
// A view inside the overlay closes it to the conversation; beside the conversation and in the full panel none does.
provide(sheetDismissKey, computed(() => (overlay.value ? () => emit('close') : null)))
/**
 * Whether a view shows in the overlay, whose own bar then ends with its Close; in its place the
 * overlay keeps a bar of its own for it, over a Host that is offline or a tab the panel cannot show.
 */
const viewShown = computed(() => props.hostOffline === null && contents.value.some((content) => content.id === selection.value))
/** The tab menu's commands that close several tabs at once. */
const closeManyItems = computed<readonly { scope: Exclude<TabCloseScope, 'self'>; label: TitleText; icon: Component }[]>(() => [
  { scope: 'right', label: t('panel.closeRight'), icon: ArrowRightToLine },
  { scope: 'left', label: t('panel.closeLeft'), icon: ArrowLeftToLine },
  { scope: 'others', label: t('panel.closeOthers'), icon: CopyX },
  { scope: 'all', label: t('panel.closeAll'), icon: ListX },
])

function closeScope(id: string, scope: TabCloseScope): void {
  emit('closeTabs', tabsToClose(props.panel.tabs, id, scope))
}

/**
 * A tab's mark as the strip draws it, as an icon of the list of every tab: its
 * spinner, its picture or its kind's mark.
 */
function listMark(mark: Component | null, data: unknown, picture: string | null, busy: boolean): FunctionalComponent {
  return () => busy
    ? h(IndeterminateSpinner, { size: ICON_PX.markIn28 })
    : picture
      ? h('img', { src: picture, alt: '', width: ICON_PX.markIn28, height: ICON_PX.markIn28, draggable: false })
      : mark ? h(mark, { data }) : null
}

/**
 * Every tab, the pinned ones first, by its mark and title, as the strip's list
 * shows them.
 */
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
    data-work-panel
    class="flex h-full min-w-0 flex-col overflow-hidden bg-surface text-fg"
    :class="[phone ? 'pb-[var(--safe-bottom)]' : 'border-l border-line', { 'pt-[var(--safe-top)]': overlay }]"
  >
    <!--
      The overlay shows no bar of its own over a view: the view's own bar
      ends with Close (`SheetCloseButton`), so Close and the view's controls
      read as one bar. Over a Host that is offline or a tab the panel cannot
      show, which have no bar, the overlay keeps one for Close. The overlay
      itself holds off the notch, which the frame's bar does in the full panel.
    -->
    <div
      v-if="!overlay"
      class="flex shrink-0 items-center gap-1"
      :class="phone ? 'h-[calc(2.75rem+var(--safe-top))] pl-1 pr-1.5 pt-[var(--safe-top)]' : 'h-11 pl-2 pr-3'"
    >
      <!--
        The chevron alone, as a phone's own back button with no room for a
        title; its name says where it goes.
      -->
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
        <!--
          The pinned tabs lead the strip in one shape whatever is selected: the
          mark, the kind's own short title and the badge, such as Change's
          counts. What the tab shows, such as the open file's name, goes in the
          tooltip, so selecting one or opening a file widens nothing. Each is a
          tab as the strip's own are: a press selects it, as do Enter and Space,
          and the arrows reach it.
        -->
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
            <!--
              The badge's smaller figures stand on the title's baseline, not
              centered beside it.
            -->
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
          :busy="hostOffline === null && (item.kind?.busy?.(item.data, item.tab.id) ?? false)"
          :icon="item.kind?.icon?.(item.data, item.tab.id) ?? null"
          @select="emit('select', item.tab.id)"
          v-context-menu="tabMenu(item.tab.id)"
          @close="emit('closeTabs', [item.tab.id])"
        >
          <template v-if="item.kind" #mark><component :is="item.kind.mark" :data="item.data" /></template>
        </TabItem>
        <!--
          With more tabs than fit, a list of every tab, as VS Code's Show Opened
          Editors: picking one selects it.
        -->
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
    <div v-else-if="!viewShown" class="flex h-11 shrink-0 items-center justify-end pr-1.5">
      <SheetCloseButton />
    </div>
    <HostOfflinePage v-if="hostOffline !== null" :name="hostOffline" />
    <div v-show="hostOffline === null" class="flex min-h-0 flex-1 flex-col overflow-hidden">
      <!--
        A host that shows only the strip, as a specimen of it does, fills the
        body itself.
      -->
      <slot>
      <!--
        Each content keeps what it shows while another tab is selected; only the
        shown one is told it is.
      -->
      <PaneScope
        v-for="content in contents"
        v-show="content.id === selection"
        :key="`${bindingOf(content.kind)}:${content.id}`"
        :name="content.name"
        :shown="content.id === selection"
        :first="firstShown.has(content.id)"
        class="flex min-h-0 flex-1 flex-col"
      >
        <component
          :is="content.kind.content"
          :tab-id="content.id"
          :data="content.data"
          :shown="content.id === selection"
          @update="(data: unknown, how?: TabUpdate) => content.pinned ? emit('updatePinned', content.id, data, how) : emit('updateTab', content.id, data, how)"
          @close="content.pinned || emit('closeTabs', [content.id])"
        />
      </PaneScope>
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
