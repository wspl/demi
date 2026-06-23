<script setup lang="ts">
import { onMounted, ref, shallowRef } from 'vue'
import { AgentWorkspace } from '@demi/web-ui/agent/workspace'
import { connectControlClient } from '@demi/web-ui/transport/control-client'
import { applyThemeToDocument } from '@demi/web-ui/theme/appTheme'
import AgentRoot from '@demi/web-ui/agent/AgentRoot.vue'

applyThemeToDocument()

const serverBase = import.meta.env.VITE_DEMI_SERVER_BASE ?? 'http://localhost:4280'

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
    <AgentRoot v-if="workspace" :workspace="workspace" />
    <div v-else class="grid h-full place-items-center text-sm text-fg-faint">{{ status }}</div>
  </div>
</template>
