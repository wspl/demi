import { reactive } from 'vue'
import type { CloudState } from '@demicodes/web-ui/cloud/types'
import { useResources } from './resources'
import { useConversations } from '../conversation/store'

/** Prototype data source; the shared CloudSettings owns the confirmation flow. */
export const cloud = reactive<CloudState>(
  {
    state: 'running',
    phase: null,
    error: null,
    systemBytes: 16 * 1024 ** 3,
    homeBytes: 32 * 1024 ** 3
  }
)
let operation: string | null = null
export async function resetCloud(operationId: string) {
  if (cloud.state === 'resetting' || operation === operationId)
    return
  operation = operationId
  cloud.state = 'resetting'
  const conversations = useConversations()
  const resources = useResources()
  for (const conversation of conversations.items) {
    const project = resources.projects.find(project => project.id === conversation.projectId)
    if (!project || project.hostKind === 'cloud') {
      conversation.stream = null
      conversation.paused = null
    }
  }
  for (const phase of ['stopping', 'saving', 'rebuilding', 'booting', 'ready'] as const) {
    cloud.phase = phase
    await new Promise(resolve => setTimeout(resolve, 500))
  }
  cloud.state = 'running'
}
