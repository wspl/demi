<script setup lang="ts">
import { computed, ref } from 'vue'
import { useSavedScroll } from '@demicodes/web-ui/composables/useSavedScroll'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import GalleryAppearanceMenu from './components/GalleryAppearanceMenu.vue'
import { useGalleryView } from './gallery-views'
import { NAV } from './router'

const route = useRoute()
const viewport = ref<HTMLElement>()
useSavedScroll(viewport, () => `demi-gallery-scroll:${route.fullPath}`)
const { views, view } = useGalleryView()

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
  <div class="flex h-full bg-surface-base text-fg">
    <aside
      class="select-none flex w-56 shrink-0 flex-col border-r border-line bg-surface"
    >
      <div class="px-4 py-4">
        <div class="text-[13px] font-medium text-fg-emphasis">Demi Gallery</div>
        <div class="mt-1 text-[12px] leading-4 text-fg-subtle">@demicodes/web-ui</div>
      </div>
      <nav class="flex flex-1 flex-col gap-0.5 px-2">
        <RouterLink
          v-for="item in NAV"
          :key="item.path"
          :to="item.path"
          class="rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors duration-200 ease-out"
          :class="route.path === item.path
            ? 'bg-active text-fg-emphasis'
            : 'text-fg-muted hover:bg-hover hover:text-fg'"
        >
          {{ item.label }}
        </RouterLink>
      </nav>
    </aside>

    <div class="flex min-w-0 flex-1 flex-col">
      <header
        class="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-line bg-surface px-5"
      >
        <Segmented v-model="view" :options="views" />
        <GalleryAppearanceMenu />
      </header>

      <main ref="viewport" :class="mainClass">
        <RouterView />
      </main>
    </div>
    <ToastHost />
  </div>
</template>
