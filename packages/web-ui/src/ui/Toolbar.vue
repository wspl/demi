<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import { ChevronsRight } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Dropdown from './Dropdown.vue'
import IconButton from './IconButton.vue'
import Menu from './Menu.vue'
import MenuDivider from './MenuDivider.vue'
import MenuItem from './MenuItem.vue'
import Tooltip from './Tooltip.vue'
import type { StatusDotTone } from './StatusDot.vue'
import { fittingActions } from './toolbar-fit'

/** One of a toolbar's actions: an icon button on the bar, a row of its More menu. */
export interface ToolbarAction {
  label: string
  icon: Component
  /** Whether a divider comes before it, starting a group. */
  group?: boolean
  pressed?: boolean
  indicator?: StatusDotTone | null
  /**
   * What a press does; a promise it answers turns the button's spinner
   * while it runs, so an action that only sends input answers none.
   */
  run: () => Promise<unknown> | void
}

/**
 * A panel's toolbar: what leads it, such as a name, in the `leading` slot,
 * then its actions. The actions that do not fit the bar's width go, from
 * its end, into a More menu (») at the end, as a macOS window's toolbar
 * puts them; the leading content takes what room the actions leave.
 */
const props = defineProps<{
  actions: readonly ToolbarAction[]
  overlayStore: OverlayStore
  /** The More menu's name. */
  moreLabel: string
}>()

const bar = ref<HTMLElement | null>(null)
/** Each action, with the divider before it, laid out or measured off the bar. */
const items = ref<HTMLElement[]>([])
const more = ref<HTMLElement | null>(null)
/** How many actions, from the start, the bar shows. */
const shown = ref(props.actions.length)

/** Shows the actions that fit; every action is laid out, those off the bar out of the flow, so each is measured. */
function fit(): void {
  const element = bar.value
  if (!element || !more.value) {
    return
  }
  const style = getComputedStyle(element)
  const gap = parseFloat(style.columnGap) || 0
  const room = element.clientWidth - (parseFloat(style.paddingLeft) || 0) - (parseFloat(style.paddingRight) || 0)
  const widths = props.actions.map((_, index) => items.value[index]?.offsetWidth ?? 0)
  shown.value = fittingActions(widths, gap, room, more.value.offsetWidth)
}

let observer: ResizeObserver | null = null
onMounted(() => {
  if (!bar.value) {
    return
  }
  observer = new ResizeObserver(() => fit())
  observer.observe(bar.value)
  fit()
})
onBeforeUnmount(() => observer?.disconnect())
// A control that comes or goes, as Android's Back, or changes its look, fits anew.
watch(() => props.actions.map((action) => `${action.label}:${action.group ?? false}`).join(), () => void nextTick(fit))
</script>

<template>
  <div ref="bar" class="relative flex h-10 flex-none items-center gap-0.5 border-b border-line px-2">
    <div class="flex min-w-0 flex-1 items-center">
      <slot name="leading" />
    </div>
    <div
      v-for="(action, index) in actions"
      :key="index"
      ref="items"
      class="flex flex-none items-center gap-0.5"
      :class="index < shown ? '' : 'invisible absolute left-0 top-0'"
      :aria-hidden="index < shown ? undefined : true"
      :inert="index >= shown || undefined"
    >
      <div v-if="action.group" class="mx-1 h-4 w-px bg-line" />
      <Tooltip :content="action.label">
        <IconButton
          :icon="action.icon"
          variant="ghost"
          size="sm"
          :pressed="action.pressed"
          :indicator="action.indicator"
          :aria-label="action.label"
          @click="action.run()"
        />
      </Tooltip>
    </div>
    <div
      ref="more"
      class="flex flex-none items-center"
      :class="shown < actions.length ? '' : 'invisible absolute left-0 top-0'"
      :aria-hidden="shown < actions.length ? undefined : true"
      :inert="shown >= actions.length || undefined"
    >
      <Dropdown :overlay-store="overlayStore">
        <template #trigger="{ isOpen }">
          <Tooltip :content="moreLabel" :disabled="isOpen">
            <IconButton
              :icon="ChevronsRight"
              variant="ghost"
              size="sm"
              :aria-label="moreLabel"
              aria-haspopup="menu"
              :pressed="isOpen"
            />
          </Tooltip>
        </template>
        <template #content>
          <Menu :aria-label="moreLabel">
            <template v-for="(action, index) in actions" :key="index">
              <template v-if="index >= shown">
                <MenuDivider v-if="action.group && index > shown" />
                <MenuItem
                  :icon="action.icon"
                  :label="action.label"
                  :indicator="action.pressed ? 'danger' : undefined"
                  @select="action.run"
                />
              </template>
            </template>
          </Menu>
        </template>
      </Dropdown>
    </div>
  </div>
</template>
