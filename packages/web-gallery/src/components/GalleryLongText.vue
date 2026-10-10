<script setup lang="ts">
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { computed, ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import MarkdownDocument from '@demicodes/web-ui/files/MarkdownDocument.vue'
import AxisPicker from './AxisPicker.vue'
import { demoModel } from '../fixtures/blocks'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { LONG_TEXT, LONG_TEXT_LANGUAGES, LONG_TEXT_NAMES } from '../fixtures/long-text'
import { alignedMarkdown, dollarMarkdown, overlongMarkdown, previewMarkdown, scriptsMarkdown } from '../fixtures/previews'

/**
 * One long reply with every kind of block, in several scripts, where a reader
 * judges the rhythm of reading text: as the transcript shows it, as the File
 * view shows the same text saved as a Markdown file, and in a column as
 * narrow as a squeezed transcript. Both set it with one style
 * (`.markdown-body`), so the two read alike.
 *
 * Edge Cases holds the rules of rendering rather than reading: a dollar
 * before an amount stays text while math renders, flush against Chinese
 * (`file-previews.md` § Markdown); Arabic joins and runs right to left in
 * code as in a sentence; a header cell starts where its column does, or as
 * the delimiter row aligns it; in the narrow column an address and a path
 * break, and a long line of code, a wide table and an equation scroll on
 * their own and fade at the side that hides more, with an overlay scrollbar
 * that takes no room.
 */
const TEXTS = [...LONG_TEXT_LANGUAGES, 'edge'] as const
type Text = (typeof TEXTS)[number]
const TEXT_NAMES: Record<Text, string> = { ...LONG_TEXT_NAMES, edge: 'Edge Cases' }
const EDGE_CASES = [previewMarkdown, dollarMarkdown, scriptsMarkdown, alignedMarkdown, overlongMarkdown].join('\n\n---\n\n')

const SURFACES = ['message', 'document', 'narrow'] as const
type Surface = (typeof SURFACES)[number]
const SURFACE_NAMES: Record<Surface, string> = { message: 'Message', document: 'Document', narrow: 'Narrow Column' }

const language = ref<Text>('zh')
const surface = ref<Surface>('message')
const text = computed(() => (language.value === 'edge' ? EDGE_CASES : LONG_TEXT[language.value]))

const createdAt = new Date(Date.now() - 120_000).toISOString()
const conversation = computed<ChatSessionState>(() => ({
  id: `long-text-${language.value}`, cwd: WORKSPACE_ROOT, title: 'Sync Stall', phase: 'idle',
  load: 'ready', lastError: null, pendingAction: null, failures: {}, archived: false, queue: [], pendingSteers: [], pendingCalls: [],
  scroll: null, subagents: [], terminals: [], shownAt: null, summaries: {},
  history: wholeHistory([
    { type: 'user', id: 'user-1', turnId: 'turn-1', model: demoModel, createdAt,
      content: [{ type: 'text', text: 'Why does the sync stall after my laptop wakes up?' }], preamble: null },
    { type: 'text', id: 'answer-1', model: demoModel, createdAt, forkable: true, text: text.value },
  ]),
}))

const place = {
  path: `${WORKSPACE_ROOT}/notes/sync-stall.md`,
  root: WORKSPACE_ROOT,
  imageUrl: (path: string) => path,
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-3 p-4">
    <div class="flex flex-wrap gap-6">
      <AxisPicker v-model="language" label="Text" :values="TEXTS" :names="TEXT_NAMES" />
      <AxisPicker v-model="surface" label="Shown As" :values="SURFACES" :names="SURFACE_NAMES" />
    </div>
    <div class="gallery-frame flex min-h-0 flex-1 bg-surface" :class="surface === 'narrow' ? 'w-80 max-w-full self-start' : ''">
      <ChatSession v-if="surface !== 'document'" :key="language" :conversation="conversation" :has-provider="true" />
      <MarkdownDocument v-else :key="language" class="flex-1" :text="text" :place="place" />
    </div>
  </div>
</template>
