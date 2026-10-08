<script setup lang="ts">
import { computed, h, nextTick, ref, watch } from 'vue'
import { useElementSize, useEventListener } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import HighlightText from '../ui/HighlightText.vue'
import Menu from '../ui/Menu.vue'
import MenuItem from '../ui/MenuItem.vue'
import Popover from '../ui/Popover.vue'
import TextInput from '../ui/TextInput.vue'
import FileIcon from './FileIcon.vue'
import { usePathCompletion, type PathCompletionKind } from './path-completion'
import type { FileBrowserEntry, FileBrowserSource } from './types'

/**
 * A text field for a Host path that offers the entries of the directory the
 * caret stands in (`usePathCompletion`): a menu under the field, as wide as
 * it, best matches first with the matched letters marked. The arrows move
 * the highlight, Tab completes the highlighted row or the first, Enter the
 * highlighted one, a click the clicked one; Escape puts the menu away, and a
 * second Escape is the field's. A directory completes as its name and a
 * slash, the menu then listing it; a file completes as its name and is
 * reported (`completeFile`). Every key the menu does not take is the
 * field's own (`keydown`).
 */
defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  source: Pick<FileBrowserSource, 'home'> & Partial<Pick<FileBrowserSource, 'showListing'>>
  /** Where a relative path starts; without it only absolute and `~/` paths complete. */
  base?: string
  kind?: PathCompletionKind
}>(), { base: undefined, kind: 'any' })

const emit = defineEmits<{
  /** A key the menu did not take, for the field's own meaning. */
  keydown: [event: KeyboardEvent]
  /** A file was completed: the text now names it. */
  completeFile: []
}>()

const model = defineModel<string>({ default: '' })

/** Rows the menu shows before it scrolls. */
const VISIBLE_ROWS = 10
/** A menu row's pitch: its height and the gap above it. */
const ROW_PITCH_PX = 29
/** The menu's padding, above and below its rows. */
const MENU_PADDING_PX = 8

const field = ref<InstanceType<typeof TextInput>>()
// Holds the menu's rows, to bring the highlighted one into view.
const rowsEl = ref<HTMLElement | null>(null)
// The field's frame: the menu hangs under it at its width.
const frame = ref<HTMLElement | null>(null)
const { width: frameWidth } = useElementSize(frame, undefined, { box: 'border-box' })

const completion = usePathCompletion({
  source: () => props.source,
  base: () => props.base,
  kind: () => props.kind,
})

const input = computed<HTMLInputElement | null>(() => field.value?.el ?? null)

watch(input, (el) => {
  frame.value = el?.closest<HTMLElement>('[data-field]') ?? null
})

/** Where the caret is, or null while the field is not focused or holds a range. */
function caretOf(el: HTMLInputElement): number | null {
  if (document.activeElement !== el || el.selectionStart !== el.selectionEnd)
    return null
  return el.selectionStart
}

/** Lets the menu follow the field's text and caret. */
function follow(): void {
  const el = input.value
  if (!el)
    return
  completion.follow(el.value, caretOf(el))
}

// The caret moves by click, arrow and Home or End too; the selection says so for each.
function onSelectionChange(): void {
  if (document.activeElement === input.value)
    follow()
}
useEventListener(document, 'selectionchange', onSelectionChange)

function onInput(value: string): void {
  model.value = value
  follow()
}

function onBlur(): void {
  completion.follow('', null)
}

/** Writes an accepted row into the field, caret at the end of its name. */
function apply(text: string, caret: number, entry: FileBrowserEntry): void {
  model.value = text
  void nextTick(() => {
    const el = input.value
    if (!el)
      return
    el.setSelectionRange(caret, caret)
    follow()
    if (!entry.isDirectory)
      emit('completeFile')
  })
}

function onKeydown(event: KeyboardEvent): void {
  const el = input.value
  const caret = el ? caretOf(el) : null
  const result = el && caret !== null
    ? completion.keydown(event.key, el.value, caret)
    : { kind: 'pass' as const }
  if (result.kind === 'pass') {
    emit('keydown', event)
    return
  }
  // The menu's key: not the form's Enter, the dialog's Escape or the focus's Tab.
  event.preventDefault()
  event.stopPropagation()
  if (result.kind === 'accept')
    apply(result.edit.text, result.edit.caret, result.entry)
}

function onRowClick(index: number): void {
  const el = input.value
  const caret = el ? caretOf(el) : null
  if (!el || caret === null)
    return
  const result = completion.accept(index, el.value, caret)
  if (result.kind === 'accept')
    apply(result.edit.text, result.edit.caret, result.entry)
}

// The highlighted row stays in sight as the arrows move it.
watch(completion.highlighted, (index) => {
  if (index < 0)
    return
  void nextTick(() => {
    rowsEl.value?.querySelectorAll('[data-menu-item]')[index]?.scrollIntoView({ block: 'nearest' })
  })
})

function iconFor(entry: FileBrowserEntry) {
  return () => h(FileIcon, { name: entry.name, isDirectory: entry.isDirectory, size: 16 })
}

const menuStyle = computed(() => ({
  width: `${frameWidth.value}px`,
  maxWidth: 'none',
  maxHeight: `min(${VISIBLE_ROWS * ROW_PITCH_PX + MENU_PADDING_PX}px, var(--overlay-available-height, 50vh))`,
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
    :aria-expanded="completion.isOpen.value"
    autocomplete="off"
    literal
    @update:model-value="onInput"
    @keydown="onKeydown"
    @click="follow"
    @focus="follow"
    @blur="onBlur"
  />
  <Popover
    :overlay-store="appOverlayStore"
    :is-open="completion.isOpen.value"
    :anchor-el="frame"
    :ignore-els="frame ? [frame] : []"
    placement="bottom-start"
    :offset="4"
    @close="completion.follow('', null)"
  >
    <!-- The field keeps the focus and its caret while a row is clicked. -->
    <div ref="rowsEl" @mousedown.prevent>
      <Menu :autofocus="false" :style="menuStyle">
        <MenuItem
          v-for="(row, index) in completion.rows.value"
          :key="row.entry.name"
          :icon="iconFor(row.entry)"
          :label="row.entry.name"
          choice
          :is-focused="index === completion.highlighted.value"
          @select="onRowClick(index)"
        >
          <span class="min-w-0 truncate"><HighlightText :text="row.entry.name" :indexes="row.indexes" />{{ row.entry.isDirectory ? '/' : '' }}</span>
        </MenuItem>
      </Menu>
    </div>
  </Popover>
</template>
