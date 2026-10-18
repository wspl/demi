<script setup lang="ts">
import { computed, ref } from 'vue'
import { useSavedScroll } from '@demicodes/web-ui/composables/useSavedScroll'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import ToastHost from '@demicodes/web-ui/ui/ToastHost.vue'
import ContextMenu from '@demicodes/web-ui/ui/ContextMenu.vue'
import SelectionToolbar from '@demicodes/web-ui/ui/SelectionToolbar.vue'
import MediaViewer from '@demicodes/web-ui/files/MediaViewer.vue'
import SidebarLayout from '@demicodes/web-ui/sidebar/SidebarLayout.vue'
import { provideBlobUrl } from '@demicodes/web-ui/agent/media-source'
import { provideMediaViewer } from '@demicodes/web-ui/files/media-viewer'
import { baseName } from '@demicodes/utils'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { RouterLink, RouterView, useRoute } from 'vue-router'
import GalleryAppearanceMenu from './components/GalleryAppearanceMenu.vue'
import GalleryLanguageMenu from './components/GalleryLanguageMenu.vue'
import { useGalleryView } from './gallery-views'
import { galleryBlobUrl } from './fixtures/blobs'
import { galleryConversationFiles } from './fixtures/message-files'
import { createGalleryWorkspace } from './fixtures/workspace'
import { productWould } from './product-would'
import { provideSendTargets, type SendTargetLook } from '@demicodes/web-ui/agent/send-to-conversation'
import { conversationTitle } from '@demicodes/web-ui/agent/conversation-title'
import { fixtureSearch, searchConversations } from './fixtures/search'
import { galleryNav } from './router'

provideBlobUrl(galleryBlobUrl)
// A transcript shown outside a session still names the gallery workspace's
// files, as a conversation's does; a file a click opens says what the product would do.
const workspace = createGalleryWorkspace()
const messageFiles = galleryConversationFiles(workspace.source, ({ path, line, root }) => {
  // A child's link or folder says the directory the panel opens for.
  const where = root === undefined ? '' : ` in ${baseName(root)}`
  productWould(path === ''
    ? `Open the Folder ${baseName(root ?? workspace.root)}`
    : `Open ${baseName(path)}${line === null ? '' : ` at Line ${line}`}${where}`)
}, ({ commit }) => productWould(`Show Commit ${commit} in Change`))
provideMessageFiles(() => ({ ...messageFiles, cwd: workspace.root }))
/** A link's Open in Work Panel: the product opens a tab of the conversation's browser. */
function openInWorkPanel(url: string): void {
  productWould(`Open ${url} in a New Tab of the Conversation’s Browser`)
}
const mediaViewer = provideMediaViewer()
// Send to Conversation lists the search window's conversations; a pick says what the product would do.
const sendable = searchConversations()
const sendLooks: Record<string, SendTargetLook> = {
  'search-ts2307': { status: { status: 'active', unread: false }, project: 'demi' },
  'search-login': { status: { status: 'done', unread: true }, project: 'demi' },
  'search-wording': { status: { status: 'idle', unread: false }, project: null },
}
provideSendTargets({
  recent: () => sendable.toSorted((a, b) => b.lastActiveAt.localeCompare(a.lastActiveAt)).map(({ messages: _, ...row }) => ({ ...row, titleRanges: [], match: null })),
  search: fixtureSearch(() => sendable),
  // Each listed conversation as its sidebar row: a state for its dot, and a project for some.
  describe: (id) => sendLooks[id] ?? null,
  shown: () => null,
  send: (to, words) => {
    const title = to === null ? 'a New Conversation' : `“${conversationTitle(sendable.find((row) => row.conversationId === to)?.title ?? null)}”`
    const files = words.files.length > 0 ? ` with ${words.files.length === 1 ? 'Its File' : `Its ${words.files.length} Files`}` : ''
    productWould(`Open ${title} with the Words at the End of Its Draft${files}`)
  },
})
const route = useRoute()
const main = ref<InstanceType<typeof ScrollArea>>()
const viewport = computed(() => main.value?.el)
useSavedScroll(viewport, () => `demi-gallery-scroll:${route.fullPath}`)
const { views, view } = useGalleryView()
const nav = computed(() => galleryNav())
// The navigation is the frame's sidebar, as the product's list is: at a
// phone's width it opens over the page, and a pick closes it.
const navOpen = ref(false)
/** The navigation's width in px; its divider drags it within the sidebar's bounds. */
const navWidth = ref(224)

// A preview fills the frame and scrolls inside itself; a page scrolls here.
// A page pads its sides and foot by the reach of the deepest floating layer's
// shadow (a dialog's: 36px to the side, 60px below), so a pinned dialog, menu
// or toast at the page's edge keeps its whole shadow inside the scroller.
const mainClass = computed(() => {
  if (route.meta.layout === 'preview' ||
    (route.path === '/session' && view.value === 'session')) {
    return 'flex flex-col'
  }
  if (route.meta.layout === 'session')
    return 'px-10 pt-4 pb-16'
  return 'px-10 pt-6 pb-16'
})
</script>

<template>
  <!-- A phone frame's page is the app alone, at the frame's size. -->
  <RouterView v-if="route.meta.layout === 'frame'" />
  <SidebarLayout
    v-else
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
        <ScrollArea class="flex-1" viewport-class="px-2 pb-2">
          <nav class="flex flex-col gap-0.5">
            <RouterLink
              v-for="item in nav"
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
        </ScrollArea>
      </aside>
    </template>

    <header
      class="flex h-10 shrink-0 items-center justify-between gap-3 border-b border-line bg-surface px-5"
    >
      <Segmented v-model="view" :options="views" />
      <div class="flex items-center gap-1">
        <GalleryLanguageMenu />
        <GalleryAppearanceMenu />
      </div>
    </header>

    <ScrollArea ref="main" class="flex-1" :viewport-class="`gallery-viewport ${mainClass}`">
      <RouterView />
    </ScrollArea>
  </SidebarLayout>
  <!-- The page's one context menu, which the root handler opens for whatever declares one. -->
  <ContextMenu :open-in-work-panel="openInWorkPanel" />
  <!-- The text selection's toolbar, by a selection in a message. -->
  <SelectionToolbar />
  <ToastHost />
  <MediaViewer :viewer="mediaViewer" />
</template>
