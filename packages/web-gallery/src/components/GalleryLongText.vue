<script setup lang="ts">
import { computed, ref } from 'vue'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import MarkdownDocument from '@demicodes/web-ui/files/MarkdownDocument.vue'
import AxisPicker from './AxisPicker.vue'
import { demoModel } from '../fixtures/blocks'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { LONG_TEXT, LONG_TEXT_LANGUAGES, LONG_TEXT_NAMES, type LongTextLanguage } from '../fixtures/long-text'

/**
 * One long reply with every kind of block, in several scripts, where a reader
 * judges the rhythm of reading text: as the transcript shows it, and as the
 * File view shows the same text saved as a Markdown file. Both set it with
 * one style (`.markdown-body`), so the two read alike.
 */
const SURFACES = ['message', 'document'] as const
type Surface = (typeof SURFACES)[number]
const SURFACE_NAMES: Record<Surface, string> = { message: 'Message', document: 'Document' }

const language = ref<LongTextLanguage>('zh')
const surface = ref<Surface>('message')
const text = computed(() => LONG_TEXT[language.value])

const createdAt = new Date(Date.now() - 120_000).toISOString()
const conversation = computed<ChatSessionState>(() => ({
  id: `long-text-${language.value}`, cwd: WORKSPACE_ROOT, title: 'Sync Stall', phase: 'idle',
  load: 'ready', lastError: null, pendingAction: null, failures: {}, archived: false, queue: [], pendingSteers: [], pendingCalls: [],
  scroll: null, subagents: [], terminals: [],
  blocks: [
    { type: 'user', id: 'user-1', turnId: 'turn-1', model: demoModel, createdAt,
      content: [{ type: 'text', text: 'Why does the sync stall after my laptop wakes up?' }], preamble: null },
    { type: 'text', id: 'answer-1', model: demoModel, createdAt, forkable: true, text: text.value },
  ],
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
      <AxisPicker v-model="language" label="Language" :values="LONG_TEXT_LANGUAGES" :names="LONG_TEXT_NAMES" />
      <AxisPicker v-model="surface" label="Shown As" :values="SURFACES" :names="SURFACE_NAMES" />
    </div>
    <div class="gallery-frame flex min-h-0 flex-1 bg-surface">
      <ChatSession v-if="surface === 'message'" :key="language" :conversation="conversation" :has-provider="true" />
      <MarkdownDocument v-else :key="language" class="flex-1 bg-surface-editor" :text="text" :place="place" />
    </div>
  </div>
</template>
