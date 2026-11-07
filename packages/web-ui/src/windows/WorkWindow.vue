<script setup lang="ts">
import { computed, h, type Component, type FunctionalComponent } from 'vue'
import { ChevronDown, ChevronLeft, Maximize2, Minimize2, Minus, PictureInPicture2, Plus, X } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import Menu from '../ui/Menu.vue'
import type { MenuListItem } from '../ui/menu-context'
import Tooltip from '../ui/Tooltip.vue'
import TabItem from '../agent/TabItem.vue'
import TabStrip from '../agent/TabStrip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import type { ContextMenuGroup } from '../ui/context-menu'
import { vContextMenu } from '../ui/context-menu'
import type { ConversationStatus } from '../agent/conversation-status'
import type { Placement } from './stage'
import { t } from './locales/text'
import { t as agentText } from '../agent/locales/text'

/**
 * A window of the conversation's work: its title bar, which is its tab bar,
 * and the selected tab's content.
 *
 * The title bar, from its start:
 *
 * - The tabs (`TabStrip`), as a web browser's title bar holds them. A press
 *   selects one; a drag reorders them. A window that only ever shows one view
 *   (`single`), such as Files, has no tab: its title bar says what it shows,
 *   the view's own icon and name, as a document window's does.
 * - New Tab, right after the last tab as in Chrome, for a window that opens
 *   its own tabs, such as the browser (`TabStrip`'s `end`).
 * - Room that takes no press but the window's: what the tabs leave in the
 *   strip, or a single view's title. Dragging it moves the window,
 *   double-clicking it maximizes or restores it.
 * - What the window adds, such as Agents' Completed (`trailing`).
 * - A divider before the controls every window has.
 * - Keep in Corner, for a window that anchors (`anchors`), pressed while it is
 *   anchored: a press anchors it, or lets it float again.
 * - Minimize, Maximize or Restore, and Close. A window docks by being dragged
 *   to the column, and leaves it the same way.
 *
 * What acts on every tab, such as Stop All, is the title bar's context menu
 * (`menu`). On a phone the window fills the area and its title bar starts
 * with Back, which minimizes it to the conversation, and has no other
 * control.
 */
export interface WindowTab {
  id: string
  title: string
  /** The tab's mark, such as a file's icon, drawn at the mark's size; the window's icon where absent. */
  mark?: Component
  /** A picture in place of the mark, such as a page's icon, by its URL. */
  icon?: string | null
  busy?: boolean
  /** Its Close, absent for a tab no Close closes, such as a running command's. */
  closable?: boolean
  /** The tab's status dot, as a command's tab shows it. */
  status?: ConversationStatus
  menu?: () => readonly ContextMenuGroup[]
}

const props = defineProps<{
  icon: Component
  name: string
  tabs: readonly WindowTab[]
  current: string | null
  placement: Placement
  focused: boolean
  /** Whether the window opens tabs of its own, with New Tab. */
  opensTabs?: boolean
  /** Whether it only ever shows its one tab, which its title bar names in place of a tab. */
  single?: boolean
  /** The rows that act on every tab, the title bar's context menu. */
  menu?: () => readonly ContextMenuGroup[]
  /** Whether it can stand anchored in the corner, with Keep in Corner. */
  anchors?: boolean
  phone?: boolean
}>()

defineSlots<{
  /** The selected tab's content. */
  default(): unknown
  /** Controls after the tabs, before the window's own. */
  trailing?(): unknown
}>()

const emit = defineEmits<{
  select: [tab: string]
  closeTab: [tab: string]
  reorder: [from: number, to: number]
  newTab: []
  /** Keep in Corner. */
  anchor: []
  minimize: []
  maximize: []
  close: []
  /** A press on the title bar's room, which moves the window. */
  grab: [event: PointerEvent]
}>()

/**
 * Every tab by its mark and title, as the strip draws them, for the list the
 * strip offers when its tabs overflow.
 */
const listedTabs = computed<MenuListItem[]>(() => props.tabs.map((tab) => {
  const mark = tab.mark ?? props.icon
  const icon: FunctionalComponent = () => tab.busy
    ? h(IndeterminateSpinner, { size: ICON_PX.markIn28 })
    : tab.icon
      ? h('img', { src: tab.icon, alt: '', width: ICON_PX.markIn28, height: ICON_PX.markIn28, draggable: false })
      : h(mark, { size: ICON_PX.markIn28 })
  return { id: tab.id, label: tab.title, icon }
}))

/** Whether a press at `target` is the window's, not a tab's or a control's. */
function onRoom(target: EventTarget | null): boolean {
  // A control drawn as a span with the button role, as IconButton is, takes its own press.
  return target instanceof Element && target.closest('[role="tab"], button, [role="button"], [data-window-control]') === null
}

function press(event: PointerEvent): void {
  if (event.button === 0 && onRoom(event.target)) {
    emit('grab', event)
  }
}

function doubleClick(event: MouseEvent): void {
  if (onRoom(event.target)) {
    emit('maximize')
  }
}
</script>

<template>
  <section
    data-work-window
    class="work-window flex h-full min-h-0 flex-col overflow-hidden bg-surface"
    :class="[placement === 'docked' ? 'rounded-lg' : 'rounded-xl', focused ? 'work-window-focused' : '']"
    :aria-label="name"
  >
    <header
      v-context-menu="menu"
      class="flex shrink-0 items-center gap-1 bg-surface-base pr-1.5 select-none"
      :class="phone ? 'h-11 pl-1' : 'h-10 pl-2'"
      @pointerdown="phone || press($event)"
      @dblclick="phone || doubleClick($event)"
    >
      <IconButton v-if="phone" :icon="ChevronLeft" variant="ghost" :aria-label="agentText('session.backToConversation')" @click="emit('minimize')" />
      <div v-if="single && tabs[0]" v-context-menu="tabs[0].menu" class="flex min-w-0 flex-1 items-center gap-1.5 px-1.5 text-chrome text-fg-emphasis">
        <img v-if="tabs[0].icon" :src="tabs[0].icon" alt="" :width="ICON_PX.markIn28" :height="ICON_PX.markIn28" class="shrink-0 rounded-sm" draggable="false">
        <component :is="tabs[0].mark ?? icon" v-else :size="ICON_PX.markIn28" class="shrink-0" />
        <span class="min-w-0 truncate">{{ tabs[0].title }}</span>
      </div>
      <TabStrip v-else class="min-w-0 flex-1" :on-reorder="(from, to) => emit('reorder', from, to)">
        <TabItem
          v-for="tab in tabs"
          :key="tab.id"
          :title="tab.title"
          :is-active="tab.id === current"
          :icon="tab.icon ?? null"
          :busy="tab.busy"
          :closable="tab.closable ?? true"
          :status="tab.status"
          v-context-menu="tab.menu"
          @select="emit('select', tab.id)"
          @close="emit('closeTab', tab.id)"
        >
          <template #mark>
            <component :is="tab.mark ?? icon" :size="ICON_PX.markIn28" />
          </template>
        </TabItem>
        <template v-if="opensTabs" #end>
          <Tooltip :content="t('newTab')" class="shrink-0">
            <IconButton :icon="Plus" variant="ghost" :aria-label="t('newTab')" @click="emit('newTab')" />
          </Tooltip>
        </template>
        <!-- With more tabs than fit, a list of every tab, as VS Code's Show Opened Editors: picking one selects it. -->
        <template #overflow>
          <Dropdown :overlay-store="appOverlayStore">
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('allTabs')">
                <IconButton :icon="ChevronDown" variant="ghost" :aria-label="t('allTabs')" :pressed="isOpen" />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu
                :items="listedTabs"
                :selected-id="current ?? undefined"
                @select="(id: string) => { emit('select', id); close() }"
              />
            </template>
          </Dropdown>
        </template>
      </TabStrip>
      <div class="work-window-actions flex shrink-0 items-center gap-1">
        <slot name="trailing" />
      </div>
      <!-- The tabs and the window's own actions stand a divider apart from the controls every window has. -->
      <span v-if="!phone" class="mx-1 h-5 w-px shrink-0 bg-line" aria-hidden="true" />
      <div v-if="!phone" class="flex shrink-0 items-center" data-window-control>
        <Tooltip v-if="anchors" :content="t('anchor')">
          <IconButton
            :icon="PictureInPicture2"
            variant="ghost"
            :aria-label="t('anchor')"
            :pressed="placement === 'anchored'"
            @click="emit('anchor')"
          />
        </Tooltip>
        <Tooltip :content="t('minimize')">
          <IconButton :icon="Minus" variant="ghost" :aria-label="t('minimize')" @click="emit('minimize')" />
        </Tooltip>
        <Tooltip :content="placement === 'maximized' ? t('restore') : t('maximize')">
          <IconButton
            :icon="placement === 'maximized' ? Minimize2 : Maximize2"
            variant="ghost"
            :aria-label="placement === 'maximized' ? t('restore') : t('maximize')"
            @click="emit('maximize')"
          />
        </Tooltip>
        <Tooltip :content="t('close')">
          <IconButton :icon="X" variant="ghost" :aria-label="t('close')" @click="emit('close')" />
        </Tooltip>
      </div>
    </header>
    <div class="relative min-h-0 flex-1 overflow-hidden">
      <slot />
    </div>
  </section>
</template>

<style scoped>
.work-window {
  box-shadow: 0 0 0 1px var(--color-line);
}

.work-window-focused {
  box-shadow: 0 0 0 1px var(--color-line-strong);
}

.work-window-actions:not(:has(button, [role="button"])) {
  display: none;
}
</style>
