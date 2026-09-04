import { join } from 'node:path'
import type { BlobStore } from '@demicodes/agent'
import type { ControlService } from './control'
import { DirBlobStore } from './blob-store'

/** Blob namespaces belong to users; a content hash never grants access across owners. */
export class UserBlobStores {
  private readonly users = new Map<string, BlobStore>()
  private readonly conversations = new Map<string, BlobStore>()

  constructor(private readonly root: string, private readonly control: ControlService) {}

  forUser(userId: string): BlobStore {
    let store = this.users.get(userId)
    if (!store) {
      if (!/^[A-Za-z0-9_-]+$/.test(userId)) throw new Error(`Invalid blob owner: ${userId}`)
      store = new DirBlobStore(join(this.root, userId))
      this.users.set(userId, store)
    }
    return store
  }

  forConversation(conversationId: string): BlobStore {
    let store = this.conversations.get(conversationId)
    if (!store) {
      let ownerStore: Promise<BlobStore> | undefined
      const resolve = () => ownerStore ??= this.control.getConversation(conversationId).then((conversation) => {
        if (!conversation) throw new Error(`No conversation ${conversationId} owns these blobs`)
        return this.forUser(conversation.userId)
      })
      store = {
        put: async (bytes) => (await resolve()).put(bytes),
        get: async (hash) => (await resolve()).get(hash),
      }
      this.conversations.set(conversationId, store)
    }
    return store
  }
}
