<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import Button from './Button.vue'
import KeyCap from './KeyCap.vue'

/**
 * A shortcut shown as key caps, with a Change button that records a new one in
 * place. While recording, the caps show whatever is held right now; a key pressed
 * with its modifiers commits, Escape cancels, and so does losing focus.
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
      class="inline-flex h-6 min-w-16 items-center justify-center rounded-md px-1 outline-none transition-[box-shadow,background-color] duration-200 ease-out"
      :class="recording ? 'bg-surface-raised ring-1 ring-line-focus' : ''"
      :aria-label="recording ? 'Recording shortcut' : undefined"
      @keydown="recording && onKeydown($event)"
      @keyup="onKeyup"
      @blur="stop"
    >
      <KeyCap v-if="shown" :keys="shown" />
      <span v-else class="select-none text-[11px] text-fg-subtle">Press keys</span>
    </span>
    <!-- mousedown would blur the field and stop recording before the click lands. -->
    <span @mousedown.prevent><Button :size="size" @click="recording ? stop() : start()">{{ recording ? 'Cancel' : 'Change' }}</Button></span>
  </span>
</template>
