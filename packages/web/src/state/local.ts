import { z } from 'zod'

const localStateSchema = z.object({
  hiddenProviders: z.array(z.string()),
  hiddenModels: z.record(z.string(), z.array(z.string())),
  recentProjects: z.array(z.string()),
  foldedProjects: z.array(z.string()),
})
export type LocalState = z.infer<typeof localStateSchema>

export function emptyLocalState(): LocalState {
  return {
    hiddenProviders: [],
    hiddenModels: {},
    recentProjects: [],
    foldedProjects: [],
  }
}

export function readLocalState(userId: string): LocalState {
  try {
    const raw = localStorage.getItem(`demi.preferences.${userId}`)
    return raw ? localStateSchema.parse(JSON.parse(raw)) : emptyLocalState()
  } catch (error) {
    // Browser storage is optional. Invalid external entries are discarded as a
    // whole; no legacy keys or partially repaired records enter product state.
    console.warn('Could not read local preferences', error)
    return emptyLocalState()
  }
}

export function writeLocalState(userId: string, value: LocalState): void {
  try {
    localStorage.setItem(`demi.preferences.${userId}`, JSON.stringify(value))
  } catch (error) {
    // Private browsing and storage quotas must not prevent server operations.
    console.warn('Could not save local preferences', error)
  }
}
