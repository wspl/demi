<script setup lang="ts">
import Button from '@demicodes/web-ui/ui/Button.vue'
import ViewLinkGone from '@demicodes/web-ui/view/ViewLinkGone.vue'
import { PageScope } from '@demicodes/web-ui/plugins/page'
import page from '@demicodes/plugin-browser'
import ViewTab from '@demicodes/plugin-browser/live/ViewTab.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'
import { galleryBrowser } from '../fixtures/live-browser'
import { productWould } from '../product-would'

/**
 * A view link's page, over the gallery's conversation browser: the one tab the
 * link names, and nothing around it. What the page tells the window that
 * frames it is a toast here.
 */
const live = galleryBrowser()
const watched = galleryBrowser()
const stopping = galleryBrowser()

/** What the page tells its parent, as `postMessage` does. */
function told(message: string): void {
  productWould(`The Page Tells Its Parent: ${message}`)
}
const handlers = {
  onLive: () => told('Live'),
  onEnded: (reason: string) => productWould(`The View Opens Again After ${reason}`),
  onClosed: () => told('Ended (Tab Closed)'),
}
</script>

<template>
  <GallerySection
    title="View Link Page"
    note="What a view link an integration made opens, without an account, in its own window or in a frame of the integration's product: one tab of a conversation's browser, live, filling the page, and nothing around it, no address bar, tab list or downloads. Clicks, keys, pastes, the page's selects and dialogs, and the right click's Copy, Cut and Paste reach the tab; Back, Forward and Reload are not offered. A read-only link shows the picture alone, with the arrow cursor, and takes nothing. Until the tab shows, the page says why in the work panel's words: connecting, a stopped Cloud, or a browser that isn't running. Once the browser runs without the tab, the page says the tab is gone and ends; a link that expired, was removed or never was says so instead of the tab. The page tells the window that frames it when the tab shows and when it ended for good."
  >
    <GallerySpecimen variant="a tab" wide>
      <div class="mb-3 flex flex-wrap gap-2">
        <Button variant="ghost" size="sm" @click="live.closeOnDevice('t1')">Close the Tab on the Device</Button>
      </div>
      <div class="gallery-frame h-[30rem]">
        <PageScope :page="page" :component="ViewTab" :props="{ open: live.stream, tab: 't1', readOnly: false, ...handlers }" />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a read-only tab" wide>
      <div class="gallery-frame h-[30rem]">
        <PageScope :page="page" :component="ViewTab" :props="{ open: watched.stream, tab: 't1', readOnly: true, ...handlers }" />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a browser that stops" wide>
      <div class="mb-3 flex flex-wrap gap-2">
        <Button variant="ghost" size="sm" @click="stopping.end()">End the Browser</Button>
      </div>
      <div class="gallery-frame h-[20rem]">
        <PageScope :page="page" :component="ViewTab" :props="{ open: stopping.stream, tab: 't1', readOnly: false, ...handlers }" />
      </div>
    </GallerySpecimen>
    <GallerySpecimen variant="a link that opens nothing" wide>
      <div class="gallery-frame h-[16rem]">
        <ViewLinkGone />
      </div>
    </GallerySpecimen>
  </GallerySection>
</template>
