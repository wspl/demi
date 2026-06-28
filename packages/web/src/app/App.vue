<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { AgentWorkspace } from '@demicodes/web-ui/agent/workspace'
import { connectControlClient } from '@demicodes/web-ui/transport/control-client'
import { applyThemeToDocument } from '@demicodes/web-ui/theme/appTheme'
import AgentRoot from '@demicodes/web-ui/agent/AgentRoot.vue'
import ThemeToggle from '@demicodes/web-ui/ui/ThemeToggle.vue'
import { WEB_BACKEND_BASE_URL } from '../dev-ports'

applyThemeToDocument()

const serverBase = WEB_BACKEND_BASE_URL

const workspace = shallowRef<AgentWorkspace | null>(null)
const status = ref('connecting…')

onMounted(async () => {
  try {
    const control = await connectControlClient(`${serverBase.replace(/^http/, 'ws')}/control`)
    const { cwd } = await control.defaultWorkspace()
    const created = new AgentWorkspace({ baseUrl: serverBase, control, cwd })
    await created.init()
    workspace.value = created
  } catch (error) {
    status.value = `error: ${error instanceof Error ? error.message : String(error)}`
  }
})
</script>

<template>
  <div class="h-full bg-surface-base">
    <ThemeToggle class="fixed right-3 top-3 z-50" />
    <AgentRoot v-if="workspace" :workspace="workspace" />
    <div v-else class="grid h-full place-items-center text-sm text-fg-faint">{{ status }}</div>
  </div>
</template>
