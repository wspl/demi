<script setup lang="ts">
import { onBeforeUnmount } from 'vue'
import SettingsInstructions from '@demicodes/web-ui/settings/SettingsInstructions.vue'

/**
 * The Instructions section over a specimen's text. A save waits a beat, as
 * the product's does, and keeps the text as the backend would: a blank one
 * removes it.
 */
const text = defineModel<string>({ required: true })

const timers = new Set<number>()
onBeforeUnmount(() => {
  for (const timer of timers) {
    window.clearTimeout(timer)
  }
})

function save(next: string): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(() => {
      timers.delete(timer)
      text.value = next.trim() === '' ? '' : next
      resolve()
    }, 400)
    timers.add(timer)
  })
}
</script>

<template>
  <SettingsInstructions :text="text" :save="save" />
</template>
