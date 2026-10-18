<script setup lang="ts">
import { computed, onBeforeUnmount } from 'vue'
import { useRoute } from 'vue-router'
import GalleryAsks from '../components/GalleryAsks.vue'

/**
 * One phone of the side questions' specimens (`GalleryPhoneFrame`): the
 * conversation at a phone's size, live, on a seed of `GalleryAsks`; its
 * selection's toolbar holds Comment, Ask and Send to Conversation, the card
 * spans the reply's width, and ↗ opens the side conversation full screen.
 * The safe areas are the iPhone 15's.
 */
const route = useRoute()
const scene = computed(() => {
  const asked = route.query.scene
  return asked === 'live' || asked === 'folded' ? asked : 'answered'
})

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
    <GalleryAsks :seed="scene" />
  </div>
</template>
