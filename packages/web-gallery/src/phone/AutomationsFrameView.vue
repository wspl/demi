<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import GalleryAutomationsApp from '../components/GalleryAutomationsApp.vue'

/**
 * One phone of the Automations page's phones (`GalleryPhoneFrame`): the app
 * frame with Automations at a phone's size, live, opened on a scene: the
 * page, an automation, a run's conversation, or a conversation automations
 * wake. The safe areas are the iPhone 15's.
 */
const route = useRoute()
// The page stays mounted while only its query changes: each scene is an app of its own.
const scene = computed(() => (typeof route.query.scene === 'string' ? route.query.scene : 'list'))

const root = document.documentElement
root.style.setProperty('--safe-top', '59px')
root.style.setProperty('--safe-bottom', '34px')
onBeforeUnmount(() => {
  root.style.removeProperty('--safe-top')
  root.style.removeProperty('--safe-bottom')
})
</script>

<template>
  <div :key="scene" class="h-dvh w-full">
    <GalleryAutomationsApp
      v-if="scene === 'automation'"
      fill
      :start="{ kind: 'automation', id: 'a-pulls' }"
    />
    <GalleryAutomationsApp
      v-else-if="scene === 'run'"
      fill
      :start="{ kind: 'run', automationId: 'a-pulls', runId: 'r-pulls-1' }"
    />
    <GalleryAutomationsApp
      v-else-if="scene === 'woken'"
      fill
      :start="{ kind: 'conversation', id: 'c-cloud-orders' }"
    />
    <GalleryAutomationsApp v-else fill />
  </div>
</template>
