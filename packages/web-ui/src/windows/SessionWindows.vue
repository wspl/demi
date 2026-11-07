<script setup lang="ts">
import { computed, h, ref, watch, type Component } from 'vue'
import { ArrowLeftToLine, ArrowRightToLine, CopyX, ListX, X } from '@lucide/vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Sheet from '../ui/Sheet.vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { usePhone } from '../ui/phone'
import type { ContextMenuGroup } from '../ui/context-menu'
import { t as shared } from '../locales/text'
import type { TabUpdate } from '../plugins/page'
import Taskbar, { type TaskbarItem, type TaskbarOpen } from './Taskbar.vue'
import WindowStage, { type StageWindowView } from './WindowStage.vue'
import WorkWindowBody from './WorkWindowBody.vue'
import type { WindowTab } from './WorkWindow.vue'
import { windowHolding, workWindows, type WorkTab, type WorkWindow } from './work-windows'
import type { PanelTabKind } from '../agent/panel-kinds/kind'
import type { SessionWork } from './session-work'
import { tabsToClose, type TabCloseScope } from '../agent/tab-close'
import { rearranged } from '../ui/drag-order'
import { windowIcon } from './taskbar-icon'
import type { WindowStageModel } from './stage'
import { t } from './locales/text'

/**
 * A conversation's windows around it: the stage, with the conversation beside
 * the column, and the taskbar under it.
 *
 * Two sources fill it:
 *
 * - The work (`work`): the conversation's tabs, grouped into their kinds'
 *   windows (`work-windows.ts`). A window whose kinds are not pinned stands
 *   while it holds a tab, minimized when one comes without being shown; a
 *   pinned kind's window stands while the user keeps it open.
 * - The session's own windows (`own`), such as Agents, Running and a side
 *   question, which the conversation's view fills (`own` slot). Those with a
 *   count stand in the taskbar's system area while it is more than none or
 *   their window is open, as a closed window with nothing to count has
 *   nothing to show.
 *
 * The page's address names the tab of the work's window in front: a tab the
 * address selects brings its window in front, opening it in the middle of the
 * area the first time; a window brought in front selects its tab there; the
 * last of the work's windows put away leaves the address naming none. A
 * conversation opened anew shows the layout it keeps.
 *
 * On a phone the taskbar gives way to a list the top bar's Tabs button opens
 * (`listOpen`), and a window shows alone over the conversation.
 */
export interface OwnWindow extends StageWindowView {
  /** The count its system button shows; absent, it is no system button and shows only while open. */
  count?: number
  select(tab: string): void
  closeTab?(tab: string): void
  /** Its window closed: it forgets the tab it showed. */
  closed?(): void
}

const props = defineProps<{
  stage: WindowStageModel
  work?: SessionWork
  own: readonly OwnWindow[]
  /** The px the composer keeps under the floating windows. */
  bottom: number
}>()

defineSlots<{
  default(): unknown
  /** The content of the session's own window `id`. */
  own(props: { id: string }): unknown
  /** What the session's own window `id` adds to its title bar. */
  trailing(props: { id: string }): unknown
  /** The system area's menus after its buttons, such as Schedules. */
  system(): unknown
  /** On a phone, the list's rows after the windows', which open what `system` opens. */
  list(): unknown
}>()

/** On a phone, the list of windows the top bar's Tabs button opens. */
const listOpen = defineModel<boolean>('listOpen', { default: false })

const phone = usePhone()
const area = ref<InstanceType<typeof WindowStage> | null>(null)

const windows = computed<WorkWindow[]>(() => (props.work ? workWindows(props.work.panel, props.work.pinned, props.work.kinds) : []))
const workIds = computed(() => new Set(windows.value.map((window) => window.window.id)))
function workOf(id: string): WorkWindow | undefined {
  return windows.value.find((window) => window.window.id === id)
}
function ownOf(id: string): OwnWindow | undefined {
  return props.own.find((window) => window.id === id)
}

/** Whether the area has its size, which places a window opening for the first time. */
const ready = computed(() => area.value?.measured ?? false)

/**
 * Opens the window `id` for the first time where `WindowStage` says, or where
 * it stood when it closed; false while the area is not measured yet.
 */
function ensure(id: string): boolean {
  if (!props.stage.find(id)) {
    if (!area.value || !ready.value) {
      return false
    }
    props.stage.add(id, area.value.opening(), ownOf(id)?.anchors ? 'anchored' : 'floating')
  }
  return true
}

/**
 * The windows whose kind opens a tab the user chose in Open's submenu, such
 * as a simulator that boots: each stands, empty, until a tab comes, the
 * opening is refused or the user closes it.
 */
const awaiting = ref<ReadonlySet<string>>(new Set())
function awaited(id: string): void {
  if (awaiting.value.has(id)) {
    const next = new Set(awaiting.value)
    next.delete(id)
    awaiting.value = next
  }
}

// A window of kinds that are not pinned stands while it holds a tab, or
// awaits one; one that comes without the address naming it waits minimized
// on the taskbar.
watch([windows, ready, awaiting], ([now]) => {
  for (const window of now) {
    if (window.tabs.some((tab) => !tab.pinned)) {
      awaited(window.window.id)
      ensure(window.window.id)
    } else if (window.tabs.length === 0 && props.stage.find(window.window.id) && !awaiting.value.has(window.window.id)) {
      props.stage.remove(window.window.id)
    }
  }
}, { immediate: true })

function awaitTab(id: string): void {
  awaiting.value = new Set([...awaiting.value, id])
  show(id)
}

/** The tab the address names, while it names one. */
const named = computed(() => {
  const work = props.work
  if (!work?.open) {
    return null
  }
  const id = work.panel.history.at(-1) ?? null
  return id !== null && windowHolding(windows.value, id) ? id : null
})
// The address names a tab: its window comes in front.
watch([named, ready], ([id]) => {
  const window = id === null ? null : windowHolding(windows.value, id)
  if (window && ensure(window.window.id)) {
    props.stage.show(window.window.id)
  }
}, { immediate: true })
/**
 * Whether the address follows this stage yet: a stage made from a kept layout
 * with a window in front names its tab in place of the address it opened on,
 * so opening the conversation is no step of the history.
 */
let following = false
watch(() => props.stage, () => {
  following = false
})
// A window of the work comes in front: the address names its tab. None in front: it names none.
watch([() => props.stage.state.focused, () => {
  const focused = props.stage.state.focused
  return focused === null ? null : workOf(focused)?.selected ?? null
}], ([focused]) => {
  const work = props.work
  if (!work) {
    return
  }
  const window = focused === null ? null : workOf(focused)
  const shown = focused !== null && props.stage.find(focused)?.placement !== 'minimized'
  if (window?.selected && shown) {
    if (named.value !== window.selected) {
      work.select(window.selected, following ? 'push' : 'replace')
    }
    following = true
  } else if (focused === null && work.open && work.tabsRead) {
    work.close()
  }
}, { immediate: true })

/** Every tab of the work, by id, as it reads now. */
const tabsById = computed(() => new Map(windows.value.flatMap((window) => window.tabs.map((tab) => [tab.id, tab] as const))))
/**
 * Each tab's mark, as a window's tab and a taskbar's list draw it: one
 * component per tab and kind, which draws the tab's data as it reads now, so
 * a tab's data read anew, as each change of the panel reads it, redraws the
 * mark and never mounts it again.
 */
const marks = new Map<string, { kind: PanelTabKind; mark: Component }>()
function markOf(tab: WorkTab): Component {
  let kept = marks.get(tab.id)
  if (kept?.kind !== tab.kind) {
    const { id, kind } = tab
    kept = {
      kind,
      mark: {
        render: () => {
          const now = tabsById.value.get(id)
          return now ? h(kind.mark, { data: now.data }) : null
        },
      },
    }
    marks.set(id, kept)
  }
  return kept.mark
}
// A closed tab's mark goes with it.
watch(tabsById, (now) => {
  for (const id of marks.keys()) {
    if (!now.has(id)) {
      marks.delete(id)
    }
  }
})

/** The tab menu's commands that close several of the window's tabs at once. */
const closeMany: readonly { scope: Exclude<TabCloseScope, 'self'>; label: () => string; icon: Component }[] = [
  { scope: 'right', label: () => t('closeRight'), icon: ArrowRightToLine },
  { scope: 'left', label: () => t('closeLeft'), icon: ArrowLeftToLine },
  { scope: 'others', label: () => t('closeOthers'), icon: CopyX },
  { scope: 'all', label: () => t('closeAll'), icon: ListX },
]

/**
 * A tab's menu: what its kind offers, then the Close commands over the
 * window's tabs, as a web browser's tab menu. A pinned tab closes by none.
 */
function tabMenu(window: WorkWindow, tab: WorkTab): (() => ContextMenuGroup[]) | undefined {
  const own = tab.kind.commands?.(tab.data, tab.id) ?? []
  if (tab.pinned && own.length === 0) {
    return undefined
  }
  const close = (ids: string[]) => props.work?.closeTabs(ids)
  return () => {
    const closable = window.tabs.filter((each) => !each.pinned)
    return [
      (tab.kind.commands?.(tab.data, tab.id) ?? []).map((command) => ({ icon: command.icon, label: command.label, disabled: command.disabled, run: () => command.run() })),
      tab.pinned ? [] : [
        { icon: X, label: shared('common.close'), run: () => close([tab.id]) },
        ...closeMany.map((item) => {
          const ids = tabsToClose(closable, tab.id, item.scope)
          return { icon: item.icon, label: item.label(), disabled: ids.length === 0, run: () => close(ids) }
        }),
      ],
    ]
  }
}

/** Whether the window only ever shows one view, its one pinned tab, such as Files: no tab of its own opens or closes. */
function isSingle(window: WorkWindow): boolean {
  return window.starts === null && window.tabs.length === 1 && window.tabs[0]!.pinned
}

/** The window's tabs; a single view's is named by what it shows, such as a file's name, as its title bar and button say. */
function workTabs(window: WorkWindow): WindowTab[] {
  const offline = props.work?.deviceOffline !== null
  const single = isSingle(window)
  return window.tabs.map((tab) => ({
    id: tab.id,
    title: single ? tab.subject : tab.title,
    mark: markOf(tab),
    icon: tab.kind.icon?.(tab.data, tab.id) ?? null,
    busy: !offline && (tab.kind.busy?.(tab.data, tab.id) ?? false),
    closable: !tab.pinned,
    menu: tabMenu(window, tab),
  }))
}

const views = computed<StageWindowView[]>(() => props.stage.state.windows.flatMap((placed): StageWindowView[] => {
  const window = workOf(placed.id)
  if (window) {
    return [{
      id: placed.id,
      icon: window.window.icon,
      name: window.window.name(),
      tabs: workTabs(window),
      current: window.selected,
      opensTabs: window.starts !== null,
      single: isSingle(window),
    }]
  }
  const own = ownOf(placed.id)
  return own ? [own] : []
}))

function select(id: string, tab: string): void {
  if (workIds.value.has(id)) {
    props.work?.select(tab)
  } else {
    ownOf(id)?.select(tab)
  }
}

function closeTab(id: string, tab: string): void {
  if (workIds.value.has(id)) {
    props.work?.closeTabs([tab])
  } else {
    ownOf(id)?.closeTab?.(tab)
  }
}

function reorder(id: string, from: number, to: number): void {
  const window = workOf(id)
  const work = props.work
  if (!window || !work) {
    return
  }
  const tabs = window.tabs.filter((tab) => !tab.pinned)
  const offset = window.tabs.length - tabs.length
  const moved = tabs[from - offset]
  const before = tabs[to - offset]
  if (moved && before) {
    work.moveTab(moved.id, work.panel.tabs.findIndex((tab) => tab.id === before.id))
  }
}

function newTab(id: string): void {
  const starts = workOf(id)?.starts
  const data = starts?.start?.()
  if (starts && data !== undefined) {
    props.work?.add(starts.kind, data)
  }
}

/** Closes the window `id`: a window of the work closes its tabs with it, as a web browser's does. */
function close(id: string): void {
  awaited(id)
  const window = workOf(id)
  if (window) {
    const tabs = window.tabs.filter((tab) => !tab.pinned).map((tab) => tab.id)
    props.stage.remove(id)
    if (tabs.length) {
      props.work?.closeTabs(tabs)
    }
    return
  }
  props.stage.remove(id)
  ownOf(id)?.closed?.()
}

/**
 * Escape on the window `id`: one that holds only what it always shows, such
 * as Files, closes, as a glance ends; one that holds the user's own tabs, such
 * as the browser's pages or a simulator, goes to the taskbar, so a key never
 * ends a page or a device.
 */
function putAway(id: string): void {
  if (workOf(id)?.tabs.some((tab) => !tab.pinned)) {
    props.stage.minimize(id)
  } else {
    close(id)
  }
}

/** Opens the window `id` from the taskbar, or the + menu: on its tab, a pinned kind's, or a new tab of a kind that opens them. */
function open(id: string): void {
  const window = workOf(id)
  if (window) {
    const tab = window.selected
    if (tab !== null) {
      ensure(id)
      props.stage.show(id)
      props.work?.select(tab)
    } else {
      newTab(id)
    }
    return
  }
  const own = ownOf(id)
  if (own) {
    ensure(id)
    props.stage.show(id)
  }
}

function press(id: string): void {
  if (props.stage.find(id)) {
    props.stage.toggle(id)
  } else {
    open(id)
  }
}

function pick(id: string, tab: string): void {
  if (!props.stage.find(id)) {
    open(id)
  }
  select(id, tab)
  props.stage.show(id)
}

function stateOf(id: string): TaskbarItem['state'] {
  const placed = props.stage.find(id)
  if (!placed) {
    return 'closed'
  }
  if (placed.placement === 'minimized') {
    return 'minimized'
  }
  return props.stage.state.focused === id ? 'front' : 'shown'
}

function itemOf(id: string): TaskbarItem | null {
  const window = workOf(id)
  if (window) {
    const tabs = workTabs(window)
    return {
      id,
      icon: window.window.icon,
      name: window.window.name(),
      tabs: props.stage.find(id) ? tabs.map((tab) => ({ id: tab.id, title: tab.title, mark: tab.mark, icon: tab.icon, closable: tab.closable })) : [],
      current: window.selected,
      pinned: props.work?.pins.includes(id) ?? false,
      state: stateOf(id),
    }
  }
  const own = ownOf(id)
  if (!own) {
    return null
  }
  return {
    id,
    icon: own.icon,
    name: own.name,
    tabs: own.tabs.map((tab) => ({ id: tab.id, title: tab.title, mark: tab.mark, icon: tab.icon, closable: tab.closable })),
    current: own.current,
    pinned: false,
    state: stateOf(id),
    count: own.count,
  }
}

/** The user's pins, then the windows that stand, in the order they came. */
const items = computed<TaskbarItem[]>(() => {
  const pins = (props.work?.pins ?? []).filter((id) => workIds.value.has(id))
  const standing = props.stage.state.windows.map((window) => window.id).filter((id) => !pins.includes(id) && ownOf(id)?.count === undefined)
  return [...pins, ...standing].flatMap((id) => itemOf(id) ?? [])
})
const system = computed<TaskbarItem[]>(() => props.own
  .flatMap((window) => window.count === undefined ? [] : itemOf(window.id) ?? [])
  .filter((item) => item.count !== 0 || item.state !== 'closed'))
/** What Open lists: every window of the work but those that say it does not (`openable`). */
const opens = computed<TaskbarOpen[]>(() => windows.value.filter((window) => window.window.openable !== false).map((window) => ({
  kind: window.window.id,
  icon: window.window.icon,
  label: window.window.name(),
  open: props.stage.find(window.window.id) !== undefined,
  newTab: window.starts !== null && props.stage.find(window.window.id) !== undefined,
  menu: window.offers?.openMenu,
})))

function openFromMenu(id: string): void {
  const window = workOf(id)
  if (window?.starts && props.stage.find(id)) {
    newTab(id)
  } else if (props.stage.find(id)) {
    props.stage.show(id)
  } else {
    open(id)
  }
}

function pin(id: string, pinned: boolean): void {
  const others = (props.work?.pins ?? []).filter((each) => each !== id)
  props.work?.keepPins(pinned ? [...others, id] : others)
}

/** The taskbar's buttons of a group dragged to a new order: the pins, or the windows that stand. */
function arrange(ids: string[], pinned: boolean): void {
  if (pinned) {
    props.work?.keepPins(rearranged(props.work.pins, ids))
  } else {
    props.stage.arrange(ids)
  }
}

function onScreen(id: string): boolean {
  const placed = props.stage.find(id)
  return placed !== undefined && placed.placement !== 'minimized' && (!phone.value || props.stage.state.focused === id)
}

/** The window `show` asked for before the area was measured, which shows once it is. */
let waiting: string | null = null
watch(ready, (measured) => {
  if (measured && waiting !== null) {
    const id = waiting
    waiting = null
    show(id)
  }
})

/** Shows the window `id` in front, opening it the first time where windows open. */
function show(id: string): void {
  if (ensure(id)) {
    props.stage.show(id)
  } else {
    waiting = id
  }
}

defineExpose({ show })

/** On a phone, the list's rows: every button the taskbar would show. */
const listed = computed(() => [...items.value, ...system.value])

function listPick(id: string): void {
  listOpen.value = false
  open(id)
}
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <div class="relative min-h-0 flex-1">
      <WindowStage
        ref="area"
        :model="stage"
        :windows="views"
        :bottom="bottom"
        :phone="phone"
        @select="select"
        @close-tab="closeTab"
        @reorder="reorder"
        @new-tab="newTab"
        @close="close"
        @escape="putAway"
      >
        <slot />
        <template #content="{ id }">
          <WorkWindowBody
            v-if="workOf(id)"
            :window="workOf(id)!"
            :on-screen="onScreen(id)"
            :device-offline="work?.deviceOffline ?? null"
            :dismiss="phone ? () => stage.minimizeAll() : null"
            @update-tab="(tab: string, data: unknown, how?: TabUpdate) => work?.updateTab(tab, data, how)"
            @update-pinned="(kind: string, data: unknown, how?: TabUpdate) => work?.updatePinned(kind, data, how)"
            @close-tab="(tab: string) => work?.closeTabs([tab])"
            @opening-failed="awaited(id)"
          />
          <slot v-else name="own" :id="id" />
        </template>
        <template #trailing="{ id }">
          <slot v-if="!workOf(id)" name="trailing" :id="id" />
        </template>
      </WindowStage>
    </div>
    <Taskbar
      v-if="!phone"
      :items="items"
      :system="system"
      :opens="opens"
      @press="press"
      @pick="pick"
      @close-tab="closeTab"
      @pin="pin"
      @arrange="arrange"
      @close="close"
      @open="openFromMenu"
      @await-tab="awaitTab"
    >
      <template #system>
        <slot name="system" />
      </template>
    </Taskbar>
    <Sheet v-if="phone" :open="listOpen" :label="t('taskbar')" :overlay-store="appOverlayStore" @close="listOpen = false">
      <Menu>
        <MenuItem
          v-for="item in listed"
          :key="item.id"
          :icon="windowIcon(item)"
          :label="item.tabs.find((tab) => tab.id === item.current)?.title ?? item.name"
          :value="item.count !== undefined ? String(item.count) : item.tabs.length > 1 ? String(item.tabs.length) : undefined"
          value-faint
          @select="listPick(item.id)"
        />
        <slot name="list" />
        <MenuItem
          v-for="entry in opens.filter((open) => open.menu || !open.open)"
          :key="`open-${entry.kind}`"
          :icon="entry.icon"
          :label="entry.label"
          @select="entry.menu || listPick(entry.kind)"
        >
          <template v-if="entry.menu" #submenu>
            <component :is="entry.menu" @opened="listOpen = false; awaitTab(entry.kind)" />
          </template>
        </MenuItem>
      </Menu>
    </Sheet>
  </div>
</template>
