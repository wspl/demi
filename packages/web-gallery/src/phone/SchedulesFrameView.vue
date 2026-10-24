<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import GallerySchedulesApp from '../components/GallerySchedulesApp.vue'

/**
 * One phone of the Schedules page's phones (`GalleryPhoneFrame`): the app
 * frame at a phone's size, live, opened on a conversation schedules wake. The
 * safe areas are the iPhone 15's.
 */
const route = useRoute()
// The page stays mounted while only its query changes: each scene is an app of its own.
const scene = computed(() => (typeof route.query.scene === 'string' ? route.query.scene : 'orders'))

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
    <GallerySchedulesApp fill :start="scene === 'price' ? 'c-price-watch' : 'c-cloud-orders'" />
  </div>
</template>
