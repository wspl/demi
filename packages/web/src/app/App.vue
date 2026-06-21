<script setup lang="ts">
import { computed, onMounted, ref, shallowRef } from 'vue'
import { AgentWorkspace } from '@demi/web-ui/agent/workspace'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import AgentMessageList from '@demi/web-ui/agent/AgentMessageList.vue'
import { applyThemeToDocument } from '@demi/web-ui/theme/appTheme'

applyThemeToDocument()

// In dev the app talks directly to the standalone server; in production the server serves this app same-origin.
const serverBase = import.meta.env.DEV ? 'http://localhost:4280' : location.origin

const workspace = shallowRef<AgentWorkspace | null>(null)
const status = ref('connecting…')
const draft = ref('')

const activeId = computed(() => workspace.value?.activeId.value ?? null)
const activeSession = computed(() => workspace.value?.activeSession.value ?? null)

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
  const id = activeId.value
  if (!current || !id || !draft.value.trim()) return
  const text = draft.value
  draft.value = ''
  await current.send(id, [{ type: 'text', text }])
}

function onContinue() {
  const current = workspace.value
  const id = activeId.value
  if (current && id) void current.resume(id)
}

function onRetry() {
  const current = workspace.value
  const id = activeId.value
  if (current && id) void current.retry(id)
}
</script>

<template>
  <div class="flex h-full flex-col bg-surface">
    <div v-if="!activeSession" class="grid h-full place-items-center text-sm text-fg-faint">{{ status }}</div>
    <template v-else>
      <div class="relative min-h-0 flex-1 overflow-hidden">
        <AgentMessageList
          :key="activeId ?? ''"
          :conversation-id="activeId ?? ''"
          :blocks="activeSession.blocks"
          :phase="activeSession.phase"
          :bottom-offset="0"
          :persisted-scroll-state="undefined"
          @continue="onContinue"
          @retry="onRetry"
        />
      </div>
      <div class="shrink-0 px-5 pb-4">
        <div class="input-float flex items-center gap-2 rounded-xl bg-surface-raised px-3 py-2 outline outline-1 outline-line">
          <input
            v-model="draft"
            placeholder="Send a message…"
            class="flex-1 bg-transparent text-sm text-fg outline-none placeholder:text-fg-subtle"
            @keydown.enter="send"
          />
          <span class="flex size-7 cursor-pointer items-center justify-center rounded-lg bg-overlay/10 text-xs text-fg transition-colors hover:bg-active" @click="send">↑</span>
        </div>
      </div>
    </template>
  </div>
</template>

<style>
.input-float {
  box-shadow: var(--shadow-float);
}
</style>
