<script setup lang="ts">
import { ref } from 'vue'
import { Pin, PinOff, TextCursorInput, Trash2 } from '@lucide/vue'
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import UserBlock from '@demicodes/web-ui/agent/blocks/UserBlock.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { vContextMenu, type ContextMenuGroup } from '@demicodes/web-ui/ui/context-menu'
import { fileMenu } from '@demicodes/web-ui/files/object-menus'
import { hostPictureCopies } from '../fixtures/attachments'
import { ago } from '../fixtures/time'
import { LINKED_COMMIT } from '../fixtures/history'
import { productWould } from '../product-would'
import GalleryAgentLinks from './GalleryAgentLinks.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * The page's context menus, each case its own specimen: right-click it, or
 * on a phone long-press it. Content adds its entries to the object around
 * it, innermost first; every entry acts on the specimen or says what the
 * product would do.
 */
const createdAt = ago(4 * 60 * 1000)

const reply = `The relogin test still reads the old cookie name. The [cookie reference](https://developer.mozilla.org/docs/Web/HTTP/Cookies) has the rename rules, and [cookie.ts](src/auth/cookie.ts) sets the new one.

![The test pattern](assets/photo.png)

\`\`\`sh
bun run test packages/web/src/auth.test.ts
\`\`\`

Select a few words of this paragraph, across two lines if you like: a right-click on them, or between their lines, keeps them selected and opens their own menu, Copy and Send to Conversation, without the message's, and on a phone the selection's toolbar stands under them with Send to Conversation. A conversation's reply adds Comment, which the toolbar offers on the desktop too (Session › Comments).`

const user = [{ type: 'text' as const, text: 'The login test fails after the session cookie rename. Can you fix it?' }]

/** A reply that links one object of each kind: a file, a commit, a running agent and a finished one, and a web page. */
const objectLinks = `The rename is in [cookie.ts](src/auth/cookie.ts), committed as [Fix the cart total](commit:${LINKED_COMMIT}). [Find the cookie assertion](agent:1) still runs, and [Update helper comments](agent:4) finished. The [cookie reference](https://developer.mozilla.org/docs/Web/HTTP/Cookies) has the rules.`

async function fork(): Promise<void> {
  productWould('Fork the Conversation from This Answer')
}

interface Row {
  id: string
  title: string
  pinned: boolean
}
const initialRows = (): Row[] => [
  { id: 'cookie', title: 'Session cookie rename', pinned: true },
  { id: 'paths', title: 'Module path aliases', pinned: false },
  { id: 'notes', title: 'Release note wording', pinned: false },
]
const rows = ref(initialRows())

// A dialog holding an object with a menu: its menu stands on the dialog and leaves it open.
const dialogOpen = ref(false)
const dialogFileMenu = (): ContextMenuGroup[] => fileMenu('/Users/alex/Projects/shop/notes.md', {
  open: () => productWould('Open notes.md'),
  showInFiles: () => productWould('Show notes.md in the File View’s Tree'),
  download: () => productWould('Download notes.md'),
})

/** A row's menu, as the sidebar's: Rename, Pin or Unpin, and Delete, which drops the row here. */
function rowMenu(row: Row): () => ContextMenuGroup[] {
  return () => [
    [
      { icon: TextCursorInput, label: 'Rename', shortcut: 'F2', run: () => productWould(`Rename “${row.title}”`) },
      { icon: row.pinned ? PinOff : Pin, label: row.pinned ? 'Unpin' : 'Pin', run: () => (row.pinned = !row.pinned) },
    ],
    [{ icon: Trash2, label: 'Delete', danger: true, run: () => (rows.value = rows.value.filter((each) => each.id !== row.id)) }],
  ]
}
</script>

<template>
  <GallerySpecimen variant="reply: link, file link, picture, code, then the message; a selection, its own menu alone" wide>
    <div class="w-[40rem] max-w-full rounded-lg bg-surface ring-1 ring-line">
      <AssistantTextBlock
        :content="reply"
        :pictures="hostPictureCopies"
        :created-at="createdAt"
        show-footer
        :fork="fork"
        :regenerate="() => productWould('Regenerate This Answer')"
      />
    </div>
  </GallerySpecimen>
  <GallerySpecimen variant="links to objects: a file’s, a commit’s and an agent’s own menu, Open first, then the message’s" wide>
    <GalleryAgentLinks :markdown="objectLinks" />
  </GallerySpecimen>
  <GallerySpecimen variant="text in the middle of a reply: no menu; a selection in it still gets the selection’s" wide>
    <div class="w-[40rem] max-w-full rounded-lg bg-surface ring-1 ring-line">
      <AssistantTextBlock content="Reading the config first, then the test." :created-at="createdAt" />
    </div>
  </GallerySpecimen>
  <GallerySpecimen variant="the user’s message: Copy Message, Edit, Send to Conversation" wide>
    <div class="w-[40rem] max-w-full rounded-lg bg-surface ring-1 ring-line">
      <UserBlock :content="user" editable @edit="productWould('Edit This Message')" />
    </div>
  </GallerySpecimen>
  <GallerySpecimen variant="rows, each its own object">
    <div class="flex w-72 flex-col gap-1 rounded-lg bg-surface-raised p-1.5 ring-1 ring-line">
      <div
        v-for="row in rows"
        :key="row.id"
        v-context-menu="rowMenu(row)"
        class="flex h-8 select-none items-center gap-2 rounded-md px-2 text-[13px] text-fg-muted hover:bg-hover"
      >
        <Pin v-if="row.pinned" :size="12" class="shrink-0 text-fg-faint" />
        {{ row.title }}
      </div>
      <Button v-if="rows.length < 3" size="sm" variant="ghost" @click="rows = initialRows()">Restore Rows</Button>
    </div>
  </GallerySpecimen>
  <GallerySpecimen variant="a menu inside a dialog leaves the dialog open">
    <Button size="sm" @click="dialogOpen = true">Open Dialog</Button>
    <Dialog :is-open="dialogOpen" :overlay-store="appOverlayStore" label="Attach Files" @close="dialogOpen = false">
      <div class="px-5 pt-5 pb-2">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Attach Files</h3>
        <p class="mt-1 text-[13px] text-fg-muted">Right-click the file: its menu opens over the dialog, and choosing leaves the dialog open.</p>
        <div
          v-context-menu="dialogFileMenu"
          class="mt-3 flex h-8 w-48 select-none items-center rounded-md px-2 text-[13px] text-fg-body hover:bg-hover"
        >
          notes.md
        </div>
      </div>
      <template #footer>
        <Button variant="primary" @click="dialogOpen = false">Done</Button>
      </template>
    </Dialog>
  </GallerySpecimen>
  <GallerySpecimen variant="blank area: no menu, the browser’s included">
    <div class="flex h-24 w-72 select-none items-center justify-center rounded-lg border border-dashed border-line text-[12px] text-fg-faint">
      Nothing declares a menu here
    </div>
  </GallerySpecimen>
</template>
