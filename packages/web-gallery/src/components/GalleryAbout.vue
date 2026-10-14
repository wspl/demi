<script setup lang="ts">
import { ref, watch } from 'vue'
import SettingsAbout from '@demicodes/web-ui/settings/SettingsAbout.vue'
import type { AcknowledgementsState } from '@demicodes/web-ui/settings/types'
import { DEMO_NOTICES, type NoticesAnswer } from '../fixtures/acknowledgements'
import { slow } from '../fixtures/slow'

/**
 * Settings › About with a stand-in server: opening Acknowledgements reads
 * the notices after a moment, and the server answers as `answer` says,
 * with the notices, with none (a development build), or with a failure,
 * whose Retry reads them again.
 */
const props = withDefaults(defineProps<{ answer?: NoticesAnswer }>(), { answer: 'notices' })

const open = ref(false)
const acknowledgements = ref<AcknowledgementsState>({ status: 'loading' })

async function load(): Promise<void> {
  acknowledgements.value = { status: 'loading' }
  await slow(600)
  acknowledgements.value = props.answer === 'notices'
    ? { status: 'ready', text: DEMO_NOTICES }
    : props.answer === 'none'
      ? { status: 'missing' }
      : { status: 'failed', error: { message: 'Could not load the acknowledgements.', requestId: null } }
}

watch(open, (isOpen) => {
  if (isOpen && acknowledgements.value.status !== 'ready') {
    void load()
  }
})
// Another answer is another server: the page reads it again when shown.
watch(() => props.answer, () => {
  acknowledgements.value = { status: 'loading' }
  if (open.value) {
    void load()
  }
})
</script>

<template>
  <SettingsAbout
    v-model:acknowledgements-open="open"
    version="0.1.21"
    :acknowledgements="acknowledgements"
    @retry="load"
  />
</template>
