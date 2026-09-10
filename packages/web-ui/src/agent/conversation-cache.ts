import { shallowReactive } from 'vue'
import type { ConversationRuntime } from './conversation-runtime'

export interface CachedConversation {
  readonly controller: AbortController
  runtime: ConversationRuntime | null
  opening: Promise<void>
}

/** Keeps opened conversations alive across navigation until explicitly invalidated. */
export class ConversationCache {
  private readonly entries = shallowReactive(new Map<string, CachedConversation>())

  get(id: string): CachedConversation | undefined {
    return this.entries.get(id)
  }

  open(
    id: string,
    initialize: (entry: CachedConversation) => Promise<void>,
  ): Promise<void> {
    const cached = this.entries.get(id)
    if (cached) {
      return cached.opening
    }
    const entry = shallowReactive<CachedConversation>({
      controller: new AbortController(),
      runtime: null,
      opening: Promise.resolve(),
    })
    this.entries.set(id, entry)
    entry.opening = Promise.resolve().then(() => {
      entry.controller.signal.throwIfAborted()
      return initialize(entry)
    }).catch((error: unknown) => {
      if (this.entries.get(id) === entry) {
        this.delete(id)
      }
      throw error
    })
    return entry.opening
  }

  delete(id: string): void {
    const entry = this.entries.get(id)
    if (!entry) {
      return
    }
    this.entries.delete(id)
    entry.controller.abort()
    entry.runtime?.dispose()
  }

  retain(ids: ReadonlySet<string>): void {
    for (const id of this.entries.keys()) {
      if (!ids.has(id)) {
        this.delete(id)
      }
    }
  }

  clear(): void {
    for (const id of this.entries.keys()) {
      this.delete(id)
    }
  }
}
