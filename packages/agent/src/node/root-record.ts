import type { AgentNodeRecord } from '../types'

export function createRootRecord(id: string): AgentNodeRecord {
  return {
    id,
    parentId: null,
    description: '',
    profileName: null,
    metadata: null,
    spawnedAt: Date.now(),
    canSpawnSubagents: true,
    closedPhase: null,
    closedAt: null,
    result: null,
    failure: null,
    delivered: false,
  }
}
