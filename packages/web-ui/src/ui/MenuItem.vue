<script setup lang="ts">
import CornerDot from './CornerDot.vue'
import StatusDot from './StatusDot.vue'
import { computed, inject, onBeforeUnmount, provide, readonly, ref, useSlots, watch } from 'vue'
import type { Component } from 'vue'
import { Check, ChevronRight } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import HighlightText from './HighlightText.vue'
import Popover from './Popover.vue'
import Tooltip from './Tooltip.vue'
import { ICON_PX } from './icon-metrics'
import { disabledTooltip } from './disabled'
import type { SentenceText, TitleText } from './ui-text'
import {
  createSubmenuController,
  menuIconlessKey,
  menuRootKey,
  menuSlotKeyboardKey,
  menuSubmenuKey,
  menuSubmenuKeysKey,
  shouldDismissMenuTree,
  type MenuSlotRow
} from './menu-context'

export type MenuIndicator = 'success' | 'warning' | 'muted' | 'accent' | 'danger'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  icon?: Component
  label?: TitleText
  value?: string
  /** A status dot: alone in the gutter, or on the icon's corner when there is one. */
  indicator?: MenuIndicator
  indicatorLabel?: SentenceText
  /** A quiet qualifier after the label, in parentheses: `Offline`, `Read-Only`. */
  note?: TitleText
  isDanger?: boolean
  disabled?: boolean
  /** Why it is disabled, as a tooltip; only read while `disabled`. */
  disabledReason?: SentenceText
  shortcut?: string
  /** A choice row: shows the check gutter, reports aria-checked, and keeps the menu tree open. */
  choice?: boolean
  isSelected?: boolean
  isFocused?: boolean
  hasSubmenu?: boolean
  iconless?: boolean
  /** Its icon and label drawn faded while it stays choosable, as a hidden file among a directory's entries. */
  faded?: boolean
  /**
   * Lit by `isFocused` alone, not by the pointer resting on it: for a list
   * whose pointer moves the selection itself, as a field's completions do,
   * so the one lit row is the one a key takes.
   */
  pointerless?: boolean
}>()

const emit = defineEmits<{
  select: []
}>()

const slots = useSlots()
const isChoice = computed(() => props.choice === true)
const isDisabled = computed(() => props.disabled === true)
const tooltipContent = computed(() => disabledTooltip(isDisabled.value, props.disabledReason))
const menuIconless = inject(
  menuIconlessKey,
  computed(() => false),
)
const menuRoot = inject(menuRootKey, null)
const showIconGutter = computed(() => props.iconless !== true && !menuIconless.value)
const showsSubmenu = computed(() => props.hasSubmenu || slots.submenu != null)
// The suffix column only exists when something renders in it, so empty rows add no width.
const hasActions = computed(() => slots.actions != null)
const hasSuffix = computed(
  () =>
    slots.suffix != null || hasActions.value || isChoice.value || showsSubmenu.value || !!props.shortcut
)

const triggerRef = ref<HTMLElement | null>(null)

// In a Menu that lays out its slot, the row its keyboard reaches: focused there, its typed prefix marked.
const keyboard = inject(menuSlotKeyboardKey, null)
const slotRow: MenuSlotRow = { label: () => props.label, el: () => triggerRef.value, enter: enterSubmenu }
const unregister = keyboard?.register(slotRow)
const keyboardFocused = computed(() => keyboard?.focused.value === slotRow)
const isFocused = computed(() => props.isFocused || keyboardFocused.value)
const typedPrefix = computed(() => (keyboardFocused.value && props.label ? keyboard?.highlight(props.label) : null) ?? undefined)
/** Pins the submenu open regardless of hover (gallery specimens). */
const pinnedOpen = defineModel<boolean>('submenuOpen', { default: false })
// The enclosing Menu arbitrates which row's submenu is open; a row rendered outside a
// Menu gets a controller of its own.
const injectedSubmenus = inject(menuSubmenuKey, null)
const ownSubmenus = injectedSubmenus ? null : createSubmenuController()
const submenus = injectedSubmenus ?? ownSubmenus!
const submenuId = Symbol('submenu')
const submenuOpen = computed(() => pinnedOpen.value || submenus.activeId.value === submenuId)
/** The keyboard opened the submenu: its menu holds the keys until it closes. */
const submenuKeys = ref(false)

/** Opens the submenu and hands it the keys, as Right Arrow or Return does on a macOS menu row. */
function enterSubmenu(): boolean {
  if (!showsSubmenu.value || isDisabled.value)
    return false
  submenus.open(submenuId)
  submenuKeys.value = true
  return true
}

/** Gives the keys back to this row's menu, the panel still showing once the submenu closes. */
function focusOwnMenu(): void {
  triggerRef.value?.closest<HTMLElement>('[role=menu]')?.focus({ preventScroll: true })
}

// However the submenu that held the keys closes (Left Arrow, Escape, the pointer opening another),
// the keys come back to this row's menu.
watch(submenuOpen, (open) => {
  if (open || !submenuKeys.value)
    return
  submenuKeys.value = false
  focusOwnMenu()
})

provide(menuSubmenuKeysKey, {
  entered: readonly(submenuKeys),
  leave: () => {
    closeSubmenu()
    focusOwnMenu()
  },
})

function openSubmenu() {
  if (!showsSubmenu.value || isDisabled.value)
    return
  submenus.open(submenuId)
}

function scheduleCloseSubmenu() {
  submenus.scheduleClose(submenuId)
}

function closeSubmenu() {
  pinnedOpen.value = false
  submenus.close(submenuId)
}

function handleClick(event: MouseEvent) {
  if (isDisabled.value) {
    event.stopPropagation()
    return
  }
  if (showsSubmenu.value) {
    openSubmenu()
    return
  }
  emit('select')
  if (
    shouldDismissMenuTree({
      isChoice: isChoice.value,
      hasSubmenu: false,
      hasSuffix: slots.suffix != null || hasActions.value,
    })
  ) {
    menuRoot?.dismiss()
  }
}

onBeforeUnmount(() => {
  submenus.close(submenuId)
  ownSubmenus?.dispose()
  unregister?.()
})

const toneClass = computed(() => {
  if (isDisabled.value)
    return 'cursor-not-allowed text-fg-faint'
  if (props.isDanger)
    return 'text-on-danger hover:bg-tint-danger-strong hover:text-on-danger'
  if (isChoice.value) {
    if (props.isSelected)
      return 'bg-active text-fg-emphasis'
    if (isFocused.value || submenuOpen.value)
      return 'bg-hover text-fg'
    return props.pointerless ? 'text-fg-body' : 'text-fg-body hover:bg-hover hover:text-fg'
  }
  if (isFocused.value || submenuOpen.value)
    return 'bg-active text-fg-emphasis'
  return 'text-fg-body hover:bg-active hover:text-fg-emphasis'
})
</script>

<template>
  <Tooltip
    v-bind="$attrs"
    data-menu-item
    class="flex shrink-0"
    :content="tooltipContent"
    :disabled="!tooltipContent"
    placement="bottom"
    :open-delay-ms="80"
    tag="div"
  >
    <div
      ref="triggerRef"
      role="menuitem"
      class="menu-row h-full w-full cursor-default select-none items-center rounded-md text-chrome transition-colors duration-200 ease-out"
      :class="toneClass"
      :aria-checked="isChoice ? isSelected : undefined"
      :aria-disabled="isDisabled || undefined"
      :aria-haspopup="showsSubmenu ? 'menu' : undefined"
      :aria-expanded="showsSubmenu ? submenuOpen : undefined"
      @click="handleClick"
      @mouseenter="openSubmenu"
      @mouseleave="scheduleCloseSubmenu"
    >
      <span
        v-if="showIconGutter"
        class="menu-cell-gutter relative flex size-4 shrink-0 items-center justify-center"
        :class="faded ? 'faded' : ''"
      >
        <component
          :is="icon"
          v-if="icon"
          :size="ICON_PX.in28"
        />
        <!-- The dot rides the icon's corner, or stands alone in the gutter. -->
        <CornerDot v-if="indicator && icon" :tone="indicator" :label="indicatorLabel" />
        <StatusDot v-else-if="indicator" :tone="indicator" :label="indicatorLabel" />
      </span>
      <span class="menu-cell-label on-fill" :class="faded ? 'faded' : ''">
        <slot>
          <span class="min-w-0 truncate"><HighlightText :text="label ?? ''" :indexes="typedPrefix" /></span>
        </slot>
        <!-- The note follows the name; the label cell is the grid column, so nothing needs to stretch. -->
        <span v-if="note" class="shrink-0 pl-1 text-fg-subtle">({{ note }})</span>
      </span>
      <span
        v-if="value"
        class="menu-cell-value truncate text-right text-fg-subtle"
        :title="value"
      >
        {{ value }}
      </span>
      <span
        v-if="hasSuffix"
        class="menu-cell-suffix flex items-center justify-end"
      >
        <!-- Trailing `xs` icon buttons go here, never in `suffix`: this cell owns their inset from the row's edge. -->
        <span v-if="hasActions" class="menu-cell-actions on-fill">
          <slot name="actions" />
        </span>
        <slot v-else name="suffix">
          <span
            v-if="isChoice"
            class="flex size-3.5 shrink-0 items-center justify-center"
          >
            <Check
              v-if="isSelected"
              :size="ICON_PX.in28"
              class="text-fg-body"
            />
          </span>
          <span
            v-else-if="showsSubmenu"
            class="flex size-3.5 shrink-0 items-center justify-center text-fg-faint"
          >
            <ChevronRight :size="ICON_PX.in28" />
          </span>
          <span
            v-else-if="shortcut"
            class="shrink-0 text-right text-[11px] text-fg-subtle"
          >
            {{ shortcut }}
          </span>
        </slot>
      </span>
    </div>
  </Tooltip>
  <!-- A submenu opens beside its item, as macOS's do, not below a trigger: its placement is its own. -->
  <Popover
    v-if="showsSubmenu"
    :overlay-store="appOverlayStore"
    :is-open="submenuOpen"
    :instant="submenus.instant.value"
    :anchor-el="triggerRef"
    placement="right-start"
    :offset="6"
    @close="closeSubmenu"
  >
    <div @mouseenter="openSubmenu" @mouseleave="scheduleCloseSubmenu">
      <slot name="submenu" />
    </div>
  </Popover>
</template>
