<script setup lang="ts">
// The frame a demo plays in. The site's page tells it which demo to play
// (`message.ts`); each time, the scene starts over from nothing.
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { isDemoId, type DemoId } from '@/features'
import DemoScene from './DemoScene.vue'
import { readPlay, tellReady } from './message'
import type { Form } from './scenes'

provideBlobUrl((ref) => ref)
provideMediaViewer()

const query = new URLSearchParams(location.search)
const asked = query.get('demo')
const form: Form = query.get('form') === 'phone' ? 'phone' : 'desktop'
const demo = ref<DemoId>(asked !== null && isDemoId(asked) ? asked : 'hero')
const run = ref(0)

const stopListening = readPlay((next) => {
  demo.value = next
  run.value += 1
})
onMounted(tellReady)
onBeforeUnmount(stopListening)
</script>

<template>
  <DemoScene :key="run" :run="run" :demo="demo" :form="form" />
</template>
