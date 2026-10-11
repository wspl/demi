<script setup lang="ts">
import { ref } from 'vue'
import GallerySection from '../components/GallerySection.vue'
import GalleryPhoneFrame from '../components/GalleryPhoneFrame.vue'

/** The scenes the page shows, each a live phone of its own. */
const CONVERSATION = [
  { scene: 'conversation', label: 'A turn running' },
  { scene: 'title', label: 'The title sheet' },
  { scene: 'drawer', label: 'The sidebar' },
  { scene: 'model', label: 'The model picker' },
]
/** How many phones have loaded: the next one loads then, so that the app's files load once, not all at once. */
const loaded = ref(0)
const PANES = [
  { scene: 'change', label: 'Work panel · Change' },
  { scene: 'file', label: 'Work panel · File' },
  { scene: 'browser', label: 'Work panel · Browser' },
  { scene: 'agents', label: 'Agents' },
  { scene: 'running', label: 'Running' },
  { scene: 'settings', label: 'Settings' },
]
</script>

<template>
  <div class="flex flex-col gap-10">
    <GallerySection
      title="The Conversation"
      note="Below 768 px the app is a phone’s: one pane at a time, and nothing of the page zooms, drifts or bounces. One 44 px bar under the status bar holds the sidebar’s button, the conversation’s title with a chevron, and the work panel’s button; the title opens the title sheet with what the desktop’s header shows beside it, Rename, Detect Title, Archive and where the conversation runs, and a dot before the title says the Host is not ready. The sidebar is a drawer from the left, 85% of the screen and at most 320 px, over a scrim that a tap closes, as a swipe left on it does. Menus open as bottom sheets with the same items: a tap on the scrim, a drag down on the grabber or the title past 30% of the sheet or a flick, Escape and Back close one, and it springs back from a shorter pull. The composer sits above the home indicator, and above the keyboard while it is open. Each phone is live: every control acts as in the product, but Back, since a page inside these frames leaves the browser’s history to the gallery around it."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in CONVERSATION"
          :key="item.scene"
          :scene="item.scene"
          :label="item.label"
          :active="index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
    <GallerySection
      title="Panels and Windows"
      note="The work panel opens full screen over the conversation, sliding in from the right, under its own bar: Conversation, which goes back to the conversation where it was, the tabs as a strip that scrolls sideways, and the strip’s controls. A file a message names opens there the same way. The Agents and Running windows rise as full-screen sheets with their tabs and Close. Back closes each, as it closes a sheet. Settings fills the screen as iOS Settings does."
    >
      <div class="flex flex-wrap gap-8">
        <GalleryPhoneFrame
          v-for="(item, index) in PANES"
          :key="item.scene"
          :scene="item.scene"
          :label="item.label"
          :active="CONVERSATION.length + index <= loaded"
          @loaded="loaded += 1"
        />
      </div>
    </GallerySection>
  </div>
</template>
