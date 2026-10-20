<script setup lang="ts" generic="T">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import { revealRow, stepHighlight } from '../composables/list-highlight'
import Menu from './Menu.vue'
import MenuItem from './MenuItem.vue'
import Popover from './Popover.vue'
import TextInput from './TextInput.vue'
import { useMenuRowHeight } from './menu-context'

/**
 * A text field that offers rows to choose from under it, as a browser's address
 * bar does: the caller owns the search and passes the rows, the field owns the
 * keyboard.
 *
 * - Typing, Down Arrow or a click opens the list with its first row lit.
 * - The arrows move the lit row and wrap, Return chooses it, the pointer moving
 *   over a row lights it and a click chooses it.
 * - Escape puts the list away and a second Escape is the surrounding dialog's.
 * - Tab leaves the field and closes the list.
 * - A field that takes the focus holding text offers its rows at once.
 * - Every key the list does not take is the field's own.
 *
 * What a row looks like is the `item` slot's. The field keeps the focus and its
 * caret while a row is clicked. Gallery: Dialogs › Add Model.
 */
defineOptions({ inheritAttrs: false })

const props = defineProps<{
  /** The rows, in the order shown: the first is lit when the list opens. */
  items: readonly T[]
  /** A row's identity, which keeps the lit row while the rows change. */
  itemKey: (item: T) => string
  /** What assistive technology calls the list. */
  listLabel: string
  /** Rows shown before the list scrolls. */
  visibleRows?: number
}>()

const emit = defineEmits<{
  /** A row was chosen with Return or a click; the list is put away. */
  choose: [item: T]
}>()

const model = defineModel<string>({ default: '' })

/** A menu row's height, and the list's padding. */
const rowHeight = useMenuRowHeight()
const MENU_PADDING_PX = 8

const field = ref<InstanceType<typeof TextInput>>()
const rowsEl = ref<HTMLElement | null>(null)
const listId = useId()
const open = ref(false)
const highlighted = ref(-1)

const input = computed<HTMLInputElement | null>(() => field.value?.el ?? null)
const frame = computed<HTMLElement | null>(() => input.value?.closest<HTMLElement>('[data-field]') ?? null)
const { width: frameWidth } = useElementSize(frame, undefined, { box: 'border-box' })

const showing = computed(() => open.value && props.items.length > 0)

function show(): void {
  open.value = true
  if (highlighted.value < 0) {
    highlighted.value = props.items.length > 0 ? 0 : -1
  }
}

function hide(): void {
  open.value = false
  highlighted.value = -1
}

// The lit row stays the same item while the rows change, else the first.
watch(
  () => props.items.map(props.itemKey),
  (keys, before) => {
    if (!open.value) {
      return
    }
    const lit = before[highlighted.value]
    const kept = lit === undefined ? -1 : keys.indexOf(lit)
    highlighted.value = kept >= 0 ? kept : keys.length > 0 ? 0 : -1
  },
)

watch(highlighted, (index) => {
  if (index < 0) {
    return
  }
  void nextTick(() => revealRow(rowsEl.value, '[data-menu-item]', index))
})

function choose(index: number): void {
  const item = props.items[index]
  if (item === undefined) {
    return
  }
  hide()
  emit('choose', item)
}

function onInput(value: string): void {
  model.value = value
  show()
}

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing) {
    return
  }
  switch (event.key) {
    case 'ArrowDown':
    case 'ArrowUp': {
      if (props.items.length === 0) {
        return
      }
      event.preventDefault()
      if (!showing.value) {
        open.value = true
        highlighted.value = event.key === 'ArrowDown' ? 0 : props.items.length - 1
        return
      }
      highlighted.value = stepHighlight(highlighted.value, event.key === 'ArrowDown' ? 1 : -1, props.items.length)
      return
    }
    case 'Enter':
      if (showing.value && highlighted.value >= 0) {
        // The list's key: not the form's Return.
        event.preventDefault()
        event.stopPropagation()
        choose(highlighted.value)
      }
      return
    case 'Escape':
      if (showing.value) {
        // The list's key: not the dialog's Escape.
        event.preventDefault()
        event.stopPropagation()
        hide()
      }
      return
    case 'Tab':
      hide()
      return
  }
}

const menuStyle = computed(() => ({
  width: `${frameWidth.value}px`,
  maxWidth: 'none',
  maxHeight: `min(${(props.visibleRows ?? 8) * (rowHeight.value + 1) + MENU_PADDING_PX}px, var(--overlay-available-height, 50vh))`,
}))

defineExpose({
  focus() {
    field.value?.focus()
  },
  select() {
    field.value?.select()
  },
})
</script>

<template>
  <TextInput
    ref="field"
    v-bind="$attrs"
    :model-value="model"
    role="combobox"
    aria-autocomplete="list"
    :aria-expanded="showing"
    :aria-controls="listId"
    autocomplete="off"
    @update:model-value="onInput"
    @keydown="onKeydown"
    @click="show"
    @focus="model && show()"
    @blur="hide"
  />
  <Popover
    :overlay-store="appOverlayStore"
    :is-open="showing"
    :anchor-el="frame"
    :ignore-els="frame ? [frame] : []"
    :offset="4"
    @close="hide"
  >
    <div ref="rowsEl" @mousedown.prevent>
      <Menu :id="listId" :aria-label="listLabel" :autofocus="false" iconless :style="menuStyle">
        <MenuItem
          v-for="(item, index) in items"
          :key="itemKey(item)"
          :label="itemKey(item)"
          choice
          pointerless
          :is-focused="index === highlighted"
          @mousemove="highlighted = index"
          @select="choose(index)"
        >
          <slot name="item" :item="item" :highlighted="index === highlighted" />
        </MenuItem>
      </Menu>
    </div>
  </Popover>
</template>
