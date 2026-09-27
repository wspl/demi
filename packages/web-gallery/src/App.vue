<script setup lang="ts">
import { computed, ref } from 'vue'
import { useSavedScroll } from '@demicodes/web-ui/composables/useSavedScroll'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import ImageViewer from '@demicodes/web-ui/files/ImageViewer.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideImageViewer } from '@demicodes/web-ui/files/image-viewer'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import GalleryAppearanceMenu from './components/GalleryAppearanceMenu.vue'
import { useGalleryView } from './gallery-views'
import { galleryBlobUrl } from './fixtures/blobs'
import { NAV } from './router'

provideBlobUrl(galleryBlobUrl)
const imageViewer = provideImageViewer()
const route = useRoute()
const viewport = ref<HTMLElement>()
useSavedScroll(viewport, () => `demi-gallery-scroll:${route.fullPath}`)
const { views, view } = useGalleryView()
// The navigation is the frame's sidebar, as the product's list is: at a
// phone's width it opens over the page, and a pick closes it.
const navOpen = ref(false)
/** The navigation's width in px; its divider drags it within the sidebar's bounds. */
const navWidth = ref(224)

const mainClass = computed(() => {
  if (route.meta.layout === 'preview' ||
    (route.path === '/session' && view.value === 'session')) {
    return 'flex min-h-0 flex-1 flex-col overflow-hidden'
  }
  if (route.meta.layout === 'session')
    return 'min-h-0 flex-1 overflow-y-auto px-5 py-4'
  return 'min-h-0 flex-1 overflow-y-auto px-6 py-6'
})
</script>

<template>
  <SidebarLayout
    v-model:open="navOpen"
    v-model:width="navWidth"
    label="Demi Gallery"
  >
    <template #sidebar>
      <aside
        class="flex h-full w-[var(--sidebar-width)] select-none flex-col border-r border-line bg-surface"
      >
        <div class="px-4 py-4">
          <div class="text-[13px] font-medium text-fg-emphasis">Demi Gallery</div>
          <div class="mt-1 text-[12px] leading-4 text-fg-subtle">@demicodes/web-ui</div>
        </div>
        <nav class="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-2">
          <RouterLink
            v-for="item in NAV"
            :key="item.path"
            :to="item.path"
            class="rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors duration-200 ease-out"
            :class="route.path === item.path
              ? 'bg-active text-fg-emphasis'
              : 'text-fg-muted hover:bg-hover hover:text-fg'"
            @click="navOpen = false"
          >
            {{ item.label }}
          </RouterLink>
        </nav>
      </aside>
    </template>

    <header
      class="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-line bg-surface px-5"
    >
      <Segmented v-model="view" :options="views" />
      <GalleryAppearanceMenu />
    </header>

    <div ref="viewport" class="gallery-viewport" :class="mainClass">
      <RouterView />
    </div>
  </SidebarLayout>
  <ToastHost />
  <ImageViewer :viewer="imageViewer" />
</template>
