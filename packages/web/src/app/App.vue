<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import type { Block } from '@demi/core'
import { AgentWorkspace } from '@demi/web-ui/agent/workspace'
import { connectControlClient } from '@demi/web-ui/transport/control-client'

// In dev the app talks directly to the standalone server; in production the server serves this app same-origin.
const serverBase = import.meta.env.DEV ? 'http://localhost:4280' : location.origin

const workspace = shallowRef<AgentWorkspace | null>(null)
const status = ref('connecting…')
const draft = ref('')

onMounted(async () => {
  try {
    const control = await connectControlClient(`${serverBase.replace(/^http/, 'ws')}/control`)
    const { cwd } = await control.defaultWorkspace()
    const created = new AgentWorkspace({ baseUrl: serverBase, control, cwd })
    await created.init()
    workspace.value = created
    status.value = `ready · cwd ${cwd}`
  } catch (error) {
    status.value = `error: ${error instanceof Error ? error.message : String(error)}`
  }
})

async function send() {
  const current = workspace.value
  const id = current?.activeId.value
  if (!current || !id || !draft.value.trim()) return
  const text = draft.value
  draft.value = ''
  await current.send(id, [{ type: 'text', text }])
}

function blockText(block: Block): string {
  if (block.type === 'text' || block.type === 'thinking') return block.text
  if (block.type === 'tool_call') return `[${block.toolName}] ${block.status}`
  if (block.type === 'user') return block.content.map((part) => (part.type === 'text' ? part.text : `[${part.type}]`)).join(' ')
  return `[${block.type}]`
}
</script>

<template>
  <div class="flex h-full flex-col gap-3 bg-surface-base p-6 text-fg">
    <header class="flex items-center gap-3">
      <h1 class="text-sm font-semibold text-fg-emphasis">Demi Web — store bootstrap</h1>
      <span class="text-xs text-fg-muted">{{ status }}</span>
    </header>

    <div v-if="workspace" class="text-xs text-fg-subtle">
      providers: {{ workspace.providers.value.map((provider) => provider.type).join(', ') || 'none' }} ·
      tabs: {{ workspace.order.value.length }}
    </div>

    <div class="min-h-0 flex-1 overflow-auto rounded-xl bg-surface p-4">
      <div
        v-for="block in workspace?.activeSession.value?.blocks ?? []"
        :key="block.id"
        class="border-b border-line-subtle py-1 text-sm"
        :class="block.type === 'user' ? 'text-fg-emphasis' : block.type === 'thinking' ? 'text-fg-subtle' : 'text-fg-body'"
      >
        <span class="mr-2 select-none text-fg-faint">{{ block.type }}</span>{{ blockText(block) }}
      </div>
      <p v-if="!(workspace?.activeSession.value?.blocks.length)" class="text-sm text-fg-faint">No messages yet.</p>
    </div>

    <div class="flex items-center gap-2 rounded-xl bg-surface-raised px-3 py-2">
      <input
        v-model="draft"
        placeholder="Send a message…"
        class="flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-fg-subtle"
        @keydown.enter="send"
      />
      <span class="cursor-pointer rounded-lg bg-active px-3 py-1 text-xs text-fg-muted" @click="send">Send</span>
    </div>
  </div>
</template>
