<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { X } from '@lucide/vue'
import Button from './Button.vue'
import IconButton from './IconButton.vue'
import KeyCap from './KeyCap.vue'
import Tooltip from './Tooltip.vue'

/**
 * A shortcut shown as key caps, with a Change button that records a new one in
 * place. While recording the button stays pressed and the caps show whatever is
 * held right now; a key pressed with its modifiers commits, Escape, losing focus
 * or clicking Change again cancels. Nothing else changes shape. An empty value is
 * "None"; the clear button beside Change, or Backspace while recording, unbinds.
 */
const props = defineProps<{
  modelValue: string
  /** Button height family; the caps have their own. */
  size?: 'sm' | 'md'
}>()

const emit = defineEmits<{
  'update:modelValue': [keys: string]
}>()

const recording = ref(false)
const held = ref('')
const field = ref<HTMLElement>()

const SPECIAL: Record<string, string> = {
  Enter: '⏎',
  Escape: '⎋',
  Backspace: '⌫',
  Delete: '⌦',
  Tab: '⇥',
  ' ': '␣',
  ArrowUp: '↑',
  ArrowDown: '↓',
  ArrowLeft: '←',
  ArrowRight: '→',
}
const MODIFIERS = new Set(['Meta', 'Control', 'Alt', 'Shift'])

function modifiers(event: KeyboardEvent): string {
  return `${event.ctrlKey ? '⌃' : ''}${event.altKey ? '⌥' : ''}${event.shiftKey ? '⇧' : ''}${event.metaKey ? '⌘' : ''}`
}

function keyCap(event: KeyboardEvent): string {
  if (MODIFIERS.has(event.key)) return ''
  if (SPECIAL[event.key]) return SPECIAL[event.key]
  // The physical key, so ⇧ doesn't turn "k" into "K" twice or "," into "<".
  if (event.code.startsWith('Key')) return event.code.slice(3)
  if (event.code.startsWith('Digit')) return event.code.slice(5)
  return event.key.length === 1 ? event.key.toUpperCase() : event.key
}

function start() {
  recording.value = true
  held.value = ''
  void nextTick(() => field.value?.focus())
}

function stop() {
  recording.value = false
  held.value = ''
}

function onKeydown(event: KeyboardEvent) {
  event.preventDefault()
  event.stopPropagation()
  if (event.key === 'Escape' && !modifiers(event)) {
    stop()
    return
  }
  if ((event.key === 'Backspace' || event.key === 'Delete') && !modifiers(event)) {
    emit('update:modelValue', '')
    stop()
    return
  }
  const cap = keyCap(event)
  held.value = modifiers(event) + cap
  if (cap) {
    emit('update:modelValue', held.value)
    stop()
  }
}

function onKeyup(event: KeyboardEvent) {
  if (!recording.value) return
  held.value = modifiers(event)
}

const shown = computed(() => (recording.value ? held.value : props.modelValue))
</script>

<template>
  <span class="inline-flex items-center gap-2">
    <span
      ref="field"
      tabindex="-1"
      class="inline-flex h-6 items-center justify-end outline-none"
      :aria-label="recording ? 'Recording shortcut' : undefined"
      @keydown="recording && onKeydown($event)"
      @keyup="onKeyup"
      @blur="stop"
    >
      <KeyCap v-if="shown" :keys="shown" />
      <span v-else class="select-none text-[11px] text-fg-subtle">{{ recording ? 'Press keys…' : 'None' }}</span>
    </span>
    <Tooltip v-if="modelValue && !recording" content="Remove shortcut"><IconButton :icon="X" :size="size" aria-label="Remove shortcut" @click="emit('update:modelValue', '')" /></Tooltip>
    <!-- mousedown would blur the field and stop recording before the click lands. -->
    <span @mousedown.prevent><Button :size="size" :pressed="recording" @click="recording ? stop() : start()">Change</Button></span>
  </span>
</template>
