<script setup lang="ts">
import { computed, inject, onBeforeUnmount, ref, useSlots } from 'vue'
import type { Component } from 'vue'
import { Check, ChevronRight } from '@lucide/vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Popover from './Popover.vue'
import Tooltip from './Tooltip.vue'
import { ICON_PX } from './icon-metrics'
import { menuIconlessKey, menuRootKey, shouldDismissMenuTree } from './menu-context'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  icon?: Component
  label?: string
  value?: string
  indicator?: 'success' | 'muted'
  indicatorLabel?: string
  isDanger?: boolean
  disabled?: boolean
  disabledReason?: string
  shortcut?: string
  /** A choice row: shows the check gutter, reports aria-checked, and keeps the menu tree open. */
  choice?: boolean
  isSelected?: boolean
  isFocused?: boolean
  hasSubmenu?: boolean
  iconless?: boolean
}>()

const emit = defineEmits<{
  select: []
}>()

const slots = useSlots()
const isChoice = computed(() => props.choice === true)
const isDisabled = computed(() => props.disabled || !!props.disabledReason)
const tooltipContent = computed(() => props.disabledReason?.trim() || undefined)
const menuIconless = inject(menuIconlessKey, computed(() => false))
const menuRoot = inject(menuRootKey, null)
const showIconGutter = computed(() => props.iconless !== true && !menuIconless.value)
const showsSubmenu = computed(() => props.hasSubmenu || slots.submenu != null)

const triggerRef = ref<HTMLElement | null>(null)
const submenuOpen = defineModel<boolean>('submenuOpen', { default: false })
let closeTimer = 0

function openSubmenu() {
  if (!showsSubmenu.value || isDisabled.value) return
  window.clearTimeout(closeTimer)
  submenuOpen.value = true
}

function scheduleCloseSubmenu() {
  window.clearTimeout(closeTimer)
  closeTimer = window.setTimeout(() => {
    submenuOpen.value = false
  }, 120)
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
  if (shouldDismissMenuTree({
    isChoice: isChoice.value,
    hasSubmenu: false,
    hasSuffix: slots.suffix != null,
  })) {
    menuRoot?.dismiss()
  }
}

onBeforeUnmount(() => {
  window.clearTimeout(closeTimer)
})

const toneClass = computed(() => {
  if (isDisabled.value) return 'cursor-not-allowed text-fg-faint'
  if (props.isDanger) return 'text-on-danger hover:bg-tint-danger-strong hover:text-on-danger'
  if (isChoice.value) {
    if (props.isSelected) return 'bg-active text-fg-emphasis'
    if (props.isFocused || submenuOpen.value) return 'bg-hover text-fg'
    return 'text-fg-muted hover:bg-hover hover:text-fg'
  }
  if (submenuOpen.value) return 'bg-active text-fg-emphasis'
  return 'text-fg-body hover:bg-active hover:text-fg-emphasis'
})
</script>

<template>
  <Tooltip
    v-bind="$attrs"
    data-menu-item
    class="flex h-7 shrink-0"
    :content="tooltipContent"
    :disabled="!tooltipContent"
    placement="bottom"
    :open-delay-ms="80"
    tag="div"
  >
    <div
      ref="triggerRef"
      role="menuitem"
      class="flex h-full w-full cursor-default select-none items-center gap-2 rounded-md px-2 text-chrome transition-colors duration-200 ease-out"
      :class="toneClass"
      :aria-checked="isChoice ? isSelected : undefined"
      :aria-haspopup="showsSubmenu ? 'menu' : undefined"
      :aria-expanded="showsSubmenu ? submenuOpen : undefined"
      @click="handleClick"
      @mouseenter="openSubmenu"
      @mouseleave="scheduleCloseSubmenu"
    >
      <span v-if="showIconGutter" class="flex size-4 shrink-0 items-center justify-center">
        <component :is="icon" v-if="icon" :size="ICON_PX.in28" />
      </span>
      <slot>
        <span class="min-w-0 flex-1 truncate">{{ label }}</span>
      </slot>
      <span v-if="value" class="max-w-[7rem] truncate text-right text-fg-muted" :title="value">
        {{ value }}
      </span>
      <span
        v-if="indicator"
        class="size-1.5 shrink-0 rounded-full"
        :class="indicator === 'success' ? 'bg-on-success' : 'bg-fg-faint'"
        role="img"
        :aria-label="indicatorLabel"
      />
      <slot name="suffix">
        <span
          v-if="isChoice"
          class="flex size-3.5 shrink-0 items-center justify-center"
        >
          <Check v-if="isSelected" :size="ICON_PX.in28" class="text-fg-body" />
        </span>
        <span
          v-else-if="showsSubmenu"
          class="flex size-3.5 shrink-0 items-center justify-center text-fg-faint"
        >
          <ChevronRight :size="ICON_PX.in28" />
        </span>
        <span
          v-else
          class="w-7 shrink-0 text-right text-[11px] text-fg-faint"
        >{{ shortcut ?? '' }}</span>
      </slot>
    </div>
  </Tooltip>
  <Popover
    v-if="showsSubmenu"
    :overlay-store="appOverlayStore"
    :is-open="submenuOpen"
    :anchor-el="triggerRef"
    placement="right-start"
    :offset="6"
    @close="submenuOpen = false"
  >
    <div @mouseenter="openSubmenu" @mouseleave="scheduleCloseSubmenu">
      <slot name="submenu" />
    </div>
  </Popover>
</template>
