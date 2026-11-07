<script setup lang="ts">
import { computed, onBeforeUnmount, provide, ref, shallowRef, watch, type Component } from 'vue'
import { useElementSize } from '@vueuse/core'
import { ChevronsUp, Pin, PinOff, Plus, X } from '@lucide/vue'
import TaskbarButton from './TaskbarButton.vue'
import { grow, shrink, TASKBAR_LEAVE_MS } from './taskbar-motion'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import MenuGroup from '../ui/MenuGroup.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import Tooltip from '../ui/Tooltip.vue'
import { vContextMenu, type ContextMenuGroup } from '../ui/context-menu'
import { appOverlayStore } from '../overlay/appOverlay'
import { useListReorder } from '../ui/useListReorder'
import { tabIcon, windowIcon } from './taskbar-icon'
import { taskbarMenuKey } from './taskbar-menu'
import { t } from './locales/text'

/**
 * The bar under the conversation that lists its work's windows, as an
 * operating system's taskbar lists its programs':
 *
 * ```text
 * | + pinned ... | open ... | More |                     | system ... |
 * ```
 *
 * - Open (+) lists what can open: a kind already open shows its window, and a
 *   kind that opens tabs opens a new one.
 * - A window's button shows its icon, the name of its shown tab and, holding
 *   more than one, their count. A press shows the window, or minimizes it while
 *   it is in front.
 * - Resting on a button lists the window's tabs over it: a press on one shows
 *   it, and each closes from its own row.
 * - Its menu pins it to the taskbar's start, after Open and before the
 *   divider, where it stays while the window is closed; or unpins it; and
 *   closes the window.
 * - Dragging a button along its group, the pinned or the open, arranges it
 *   among them (`arrange`), as an operating system's taskbar does.
 * - Buttons narrow to a least width as more come; those that do not fit then
 *   are listed under More.
 * - The system's own buttons stand at the end, after a divider, in their
 *   order: they are not pinned or unpinned, show their count in place of a
 *   title, and their menus open leftward from there. The `system` slot adds
 *   buttons there that open a menu, such as Schedules.
 */
export interface TaskbarTab {
  id: string
  title: string
  mark?: Component
  /** A picture in place of the mark, such as a page's icon, by its URL. */
  icon?: string | null
  /** Its Close, absent for a tab no Close closes, such as a running command's. */
  closable?: boolean
}

export interface TaskbarItem {
  id: string
  icon: Component
  /** The window's name, which a closed pinned button shows. */
  name: string
  /** What the window shows: none while it is closed. */
  tabs: readonly TaskbarTab[]
  current: string | null
  pinned: boolean
  /** closed: pinned, its window not open; front: the window takes the keys. */
  state: 'closed' | 'minimized' | 'shown' | 'front'
  /** The count a system button shows in place of a title. */
  count?: number
}

export interface TaskbarOpen {
  kind: string
  icon: Component
  label: string
  /** Whether a window of the kind is open already. */
  open: boolean
  /** Whether choosing it opens a new tab in its window. */
  newTab?: boolean
}

const props = defineProps<{
  items: readonly TaskbarItem[]
  system: readonly TaskbarItem[]
  opens: readonly TaskbarOpen[]
}>()

const emit = defineEmits<{
  press: [id: string]
  pick: [id: string, tab: string]
  closeTab: [id: string, tab: string]
  pin: [id: string, pinned: boolean]
  /** The buttons of a group, pinned or not, in the order the user dragged them to. */
  arrange: [ids: string[], pinned: boolean]
  close: [id: string]
  open: [kind: string]
}>()

/** The least width a button narrows to, in px: its icon and a few characters. */
const BUTTON_MIN_PX = 104
/** The most width a button takes, in px: a tab's name reads, and a few buttons still fit beside the system area. */
const BUTTON_MAX_PX = 200
/** The space between two buttons, in px. */
const GAP_PX = 2
/** The width of the More button the buttons that do not fit go into, in px. */
const MORE_PX = 76

const list = ref<HTMLElement | null>(null)
const { width: room } = useElementSize(list)

const pinned = computed(() => props.items.filter((item) => item.pinned))
const unpinned = computed(() => props.items.filter((item) => !item.pinned))
const fits = computed(() => {
  const all = unpinned.value.length
  if (room.value === 0 || all * (BUTTON_MIN_PX + GAP_PX) <= room.value) {
    return all
  }
  return Math.max(1, Math.floor((room.value - MORE_PX) / (BUTTON_MIN_PX + GAP_PX)))
})
const shown = computed(() => unpinned.value.slice(0, fits.value))
const overflow = computed(() => unpinned.value.slice(fits.value))

const pinnedList = ref<HTMLElement | null>(null)
const pinnedOrder = useListReorder(pinnedList, () => pinned.value.map((item) => item.id), (ids) => emit('arrange', ids, true), 'x')
// The buttons under More stay after the ones shown, where they are.
const shownOrder = useListReorder(
  list,
  () => shown.value.map((item) => item.id),
  (ids) => emit('arrange', [...ids, ...overflow.value.map((item) => item.id)], false),
  'x',
)
// A button dragged shows no tabs over it.
watch(() => pinnedOrder.dragging.value ?? shownOrder.dragging.value, (dragged) => {
  if (dragged !== null) {
    dismiss()
  }
})

function label(item: TaskbarItem): string {
  return item.tabs.find((tab) => tab.id === item.current)?.title ?? item.tabs[0]?.title ?? item.name
}

function menu(item: TaskbarItem, system: boolean): () => ContextMenuGroup[] {
  return () => {
    const groups: ContextMenuGroup[] = []
    if (!system) {
      groups.push([{ icon: item.pinned ? PinOff : Pin, label: item.pinned ? t('unpin') : t('pin'), run: () => emit('pin', item.id, !item.pinned) }])
    }
    if (item.state !== 'closed') {
      groups.push([{ icon: X, label: t('closeWindow'), run: () => emit('close', item.id) }])
    }
    return groups
  }
}

/** The button whose tabs the menu over it lists; a system button's opens leftward, from the taskbar's end. */
const listed = shallowRef<{ id: string; anchor: HTMLElement; system: boolean } | null>(null)
const listedItem = computed(() => [...props.items, ...props.system].find((item) => item.id === listed.value?.id) ?? null)
const menuOwner = ref<string | null>(null)
provide(taskbarMenuKey, menuOwner)
watch(listed, (now) => {
  if (now) {
    menuOwner.value = now.id
  }
})
// Another button's menu opened: this one's tabs go at once.
watch(menuOwner, (owner) => {
  if (listed.value && owner !== listed.value.id) {
    dismiss()
  }
})
let opening: ReturnType<typeof setTimeout> | null = null
let closing: ReturnType<typeof setTimeout> | null = null

/** How long the pointer rests on a button before its tabs show, in ms; moving to the next button while they show is at once. */
const REST_MS = 400

/**
 * The pointer rests on `item`'s button: its tabs show over it, a window's
 * after a moment, as a taskbar's thumbnails do, and a system button's at
 * once, as its menu, even while its window is closed, a pick then opening it.
 */
function rest(item: TaskbarItem, event: PointerEvent, system = false): void {
  if (pinnedOrder.dragging.value !== null || shownOrder.dragging.value !== null) {
    return
  }
  if (closing) {
    clearTimeout(closing)
    closing = null
  }
  if (opening) {
    clearTimeout(opening)
  }
  if (item.tabs.length === 0 || (item.state === 'closed' && !system)) {
    leave()
    return
  }
  const anchor = event.currentTarget as HTMLElement
  const place = () => {
    listed.value = { id: item.id, anchor, system }
  }
  if (listed.value || system) {
    place()
  } else {
    opening = setTimeout(place, REST_MS)
  }
}

function leave(): void {
  if (opening) {
    clearTimeout(opening)
    opening = null
  }
  closing ??= setTimeout(() => {
    listed.value = null
    closing = null
  }, TASKBAR_LEAVE_MS)
}

function keep(): void {
  if (closing) {
    clearTimeout(closing)
    closing = null
  }
}

function dismiss(): void {
  if (opening) {
    clearTimeout(opening)
    opening = null
  }
  listed.value = null
}

onBeforeUnmount(() => {
  if (opening) {
    clearTimeout(opening)
  }
  if (closing) {
    clearTimeout(closing)
  }
})

function press(item: TaskbarItem): void {
  dismiss()
  emit('press', item.id)
}
</script>

<template>
  <nav
    data-taskbar
    :aria-label="t('taskbar')"
    class="flex h-11 shrink-0 items-center gap-0.5 border-t border-line-subtle bg-surface px-1.5"
  >
    <!-- Disabled while nothing can open, as in a conversation not yet started. -->
    <Dropdown :overlay-store="appOverlayStore" side="top" align="start" :disabled="opens.length === 0">
      <template #trigger="{ isOpen }">
        <Tooltip :content="t('open')">
          <IconButton :icon="Plus" variant="ghost" :aria-label="t('open')" :pressed="isOpen" :disabled="opens.length === 0" />
        </Tooltip>
      </template>
      <!-- A chosen row closes the menu itself. -->
      <template #content>
        <Menu>
          <MenuItem
            v-for="entry in opens"
            :key="entry.kind"
            :icon="entry.icon"
            :label="entry.label"
            :value="entry.newTab ? t('newTab') : entry.open ? t('alreadyOpen') : undefined"
            value-faint
            @select="emit('open', entry.kind)"
          />
        </Menu>
      </template>
    </Dropdown>
    <div ref="pinnedList" class="contents">
      <TransitionGroup :css="false" @enter="grow" @leave="shrink">
        <TaskbarButton
          v-for="item in pinned"
          :key="item.id"
          :data-reorder-id="item.id"
          :icon="windowIcon(item)"
          :label="label(item)"
          :count="item.state !== 'closed' && item.tabs.length > 1 ? item.tabs.length : undefined"
          :state="item.state"
          :style="{ minWidth: `min(${BUTTON_MIN_PX}px, max-content)`, maxWidth: `${BUTTON_MAX_PX}px` }"
          v-context-menu="menu(item, false)"
          @contextmenu="dismiss"
          @click="press(item)"
          @pointerdown="pinnedOrder.start($event, item.id)"
          @pointerenter="rest(item, $event)"
          @pointerleave="leave"
        />
      </TransitionGroup>
    </div>
    <span class="mx-1 h-5 w-px shrink-0 bg-line" aria-hidden="true" />
    <div ref="list" class="flex min-w-0 flex-1 items-center gap-0.5 overflow-hidden">
      <TransitionGroup :css="false" @enter="grow" @leave="shrink">
        <TaskbarButton
          v-for="item in shown"
          :key="item.id"
          :data-reorder-id="item.id"
          :icon="windowIcon(item)"
          :label="label(item)"
          :count="item.state !== 'closed' && item.tabs.length > 1 ? item.tabs.length : undefined"
          :state="item.state"
          :style="{ minWidth: `min(${BUTTON_MIN_PX}px, max-content)`, maxWidth: `${BUTTON_MAX_PX}px` }"
          v-context-menu="menu(item, false)"
          @contextmenu="dismiss"
          @click="press(item)"
          @pointerdown="shownOrder.start($event, item.id)"
          @pointerenter="rest(item, $event)"
          @pointerleave="leave"
        />
      </TransitionGroup>
      <Dropdown v-if="overflow.length" :overlay-store="appOverlayStore" side="top" align="start">
        <template #trigger="{ isOpen }">
          <TaskbarButton :icon="ChevronsUp" :label="t('more')" :state="isOpen ? 'front' : undefined" fixed />
        </template>
        <template #content="{ close }">
          <Menu @click="close">
            <MenuItem
              v-for="item in overflow"
              :key="item.id"
              :icon="windowIcon(item)"
              :label="label(item)"
              :value="item.tabs.length > 1 ? String(item.tabs.length) : undefined"
              value-faint
              @select="emit('press', item.id)"
            />
          </Menu>
        </template>
      </Dropdown>
    </div>
    <span v-if="system.length || $slots.system" class="mx-1 h-5 w-px shrink-0 bg-line" aria-hidden="true" />
    <!-- No box of its own: the buttons stand in the taskbar's row, with its gaps. -->
    <div class="contents">
      <TransitionGroup :css="false" @enter="grow" @leave="shrink">
        <!-- No tip: resting on one shows its menu at once, which its name heads. -->
        <TaskbarButton
          v-for="item in system"
          :key="item.id"
          :icon="item.icon"
          :count="item.count ?? item.tabs.length"
          :state="item.state"
          :aria-label="item.name"
          fixed
          v-context-menu="menu(item, true)"
          @contextmenu="dismiss"
          @click="press(item)"
          @pointerenter="rest(item, $event, true)"
          @pointerleave="leave"
        />
      </TransitionGroup>
      <slot name="system" />
    </div>
    <!-- The window's tabs, a menu over its button while the pointer rests there, as a taskbar's thumbnails list a program's windows. -->
    <Popover
      :overlay-store="appOverlayStore"
      :is-open="listed !== null && listedItem !== null"
      :anchor-el="listed?.anchor"
      side="top"
      :align="listed?.system ? 'end' : 'start'"
      @close="dismiss"
    >
      <Menu v-if="listed && listedItem" class="min-w-56 max-w-80" :autofocus="false" @pointerenter="keep" @pointerleave="leave">
        <MenuGroup :label="listedItem.name">
          <MenuItem
            v-for="tab in listedItem.tabs"
            :key="tab.id"
            :icon="tabIcon(tab, listedItem.icon)"
            :label="tab.title"
            choice
            :is-selected="tab.id === listedItem.current"
            @select="emit('pick', listedItem.id, tab.id); dismiss()"
          >
            <template v-if="listedItem.tabs.length > 1 && tab.closable !== false" #actions>
              <IconButton :icon="X" size="xs" variant="ghost" :aria-label="t('close')" @click.stop="emit('closeTab', listedItem.id, tab.id)" />
            </template>
          </MenuItem>
        </MenuGroup>
      </Menu>
    </Popover>
  </nav>
</template>
