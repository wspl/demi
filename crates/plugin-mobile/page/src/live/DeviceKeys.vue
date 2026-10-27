<script setup lang="ts">
import { ref } from 'vue'
import { keyInput } from '@demicodes/plugin-sdk'
import type { DeviceSession } from './session'
import { insertedText } from './typed'
import { t } from '../locales/text'

/**
 * The keyboard of a device's view: a field of no size that takes the
 * viewer's keys and text for the device while it has the focus, which a
 * touch on the device gives it. It sits where the viewer touched, so an
 * input method's candidate window opens there. A key on a screen that is
 * off wakes it, as on the device.
 */
const props = defineProps<{
  session: DeviceSession
  screenOff: boolean
}>()

const field = ref<HTMLInputElement | null>(null)

/** The field takes the keys, at `x`, `y` of its area, within `width` × `height`. */
function focusAt(x: number, y: number, width: number, height: number): void {
  if (!field.value) {
    return
  }
  field.value.style.left = `${Math.max(0, Math.min(width - 2, x))}px`
  field.value.style.top = `${Math.max(0, Math.min(height - 2, y))}px`
  field.value.focus({ preventScroll: true })
}

defineExpose({ focusAt })

function key(event: KeyboardEvent, action: 'down' | 'up'): void {
  if (event.isComposing) {
    return
  }
  event.preventDefault()
  if (props.screenOff) {
    if (action === 'down' && !event.repeat) {
      props.session.wake()
    }
    return
  }
  props.session.input(keyInput(action, event))
}

function inserted(event: Event): void {
  if (!(event instanceof InputEvent)) {
    return
  }
  const text = insertedText(event)
  if (text) {
    props.session.input({ type: 'text', text })
  }
  if (field.value && !event.isComposing) {
    field.value.value = ''
  }
}

function composed(event: CompositionEvent): void {
  if (event.data) {
    props.session.input({ type: 'text', text: event.data })
  }
  if (field.value) {
    field.value.value = ''
  }
}
</script>

<template>
  <input
    ref="field"
    class="pointer-events-none absolute left-0 top-0 h-[2px] w-[2px] border-0 bg-transparent p-0 text-transparent caret-transparent outline-none"
    :aria-label="t('keyboard')"
    autocomplete="off"
    autocorrect="off"
    spellcheck="false"
    @keydown="key($event, 'down')"
    @keyup="key($event, 'up')"
    @input="inserted"
    @compositionend="composed"
    @blur="session.release()"
  >
</template>
