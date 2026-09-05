import { defineStore } from 'pinia'
import type { Block, UserContentBlock } from '@demicodes/core'
import { conversation, conversations, modelSelection } from '../prototype/fixtures'
import type { Conversation } from '../prototype/types'

function meta(c: Conversation) {
  return {
    id: crypto.randomUUID(),
    createdAt: new Date().toISOString(),
    model: modelSelection(c),
  }
}

export const useConversations = defineStore('conversations', {
  state: () => ({ items: conversations(), tabs: ['welcome'], notice: '', failNext: false }),
  actions: {
    create(projectId: string | null = null): string {
      const c = conversation(crypto.randomUUID(), 'New conversation', projectId)
      this.items.unshift(c)
      this.open(c.id)
      return c.id
    },
    open(id: string) {
      if (!this.tabs.includes(id)) this.tabs.push(id)
    },
    close(id: string) {
      this.tabs = this.tabs.filter((tab) => tab !== id)
    },
    rename(id: string, title: string) {
      const c = this.items.find((item) => item.id === id)
      if (c && title.trim()) c.title = title.trim()
    },
    archive(ids: string[], archived = true) {
      for (const c of this.items.filter((item) => ids.includes(item.id))) {
        if (c.stream) {
          this.notice = 'Stop the running turn before archiving this conversation.'
          continue
        }
        c.archived = archived
        if (archived) this.close(c.id)
      }
    },
    move(ids: string[], projectId: string | null) {
      for (const c of this.items.filter((item) => ids.includes(item.id))) {
        if (c.stream) {
          this.notice = 'Wait for the turn to finish before changing its environment.'
          continue
        }
        c.projectId = projectId
      }
    },
    send(c: Conversation) {
      if (c.archived || (!c.draft.trim() && !c.files.length)) return
      const content: UserContentBlock[] = []
      const fileReferences = c.files
        .filter((f) => !f.src || f.destination === 'workspace')
        .map((f) => `${f.destination === 'workspace' ? 'Workspace file' : 'Attachment'}: ${f.name}`)
      const text = [c.draft.trim(), ...fileReferences].filter(Boolean).join('\n\n')
      content.push({ type: 'text', text })
      for (const file of c.files)
        if (file.src && file.destination === 'message')
          content.push({ type: 'image', source: { type: 'url', url: file.src } })
      if (c.stream) {
        if (c.files.length) {
          this.notice = 'Use a text-only draft to queue the next turn.'
          return
        }
        c.queue.push({ id: crypto.randomUUID(), text, content })
        c.draft = ''
        return
      }
      if (!c.blocks.length)
        c.title = (c.draft.trim() || c.files[0]?.name || 'New conversation').slice(0, 70)
      c.blocks.push({
        ...meta(c),
        type: 'user',
        turnId: crypto.randomUUID(),
        content,
        preamble: null,
      })
      c.draft = ''
      c.files = []
      this.start(c)
    },
    start(c: Conversation) {
      if (c.stream || c.archived) return
      const block: Block = { ...meta(c), type: 'text', text: '' }
      c.blocks.push(block)
      c.status = 'active'
      c.updatedAt = new Date().toISOString()
      c.stream = {
        blockId: block.id,
        remaining:
          'Here’s a way to approach this.\n\n1. **Make the outcome concrete.** Describe what a useful result looks like.\n2. **Work through one piece at a time.** Keep the next step small enough to review.\n3. **Check the result together.** Adjust what needs attention before moving on.\n\nYou can keep this conversation open while you explore another project. This response is a scripted preview of the conversation experience.',
        fail: this.failNext,
      }
      this.failNext = false
    },
    advance() {
      for (const c of this.items) {
        const stream = c.stream
        if (!stream) continue
        if (stream.fail) {
          c.blocks = c.blocks.filter((b) => b.id !== stream.blockId)
          c.blocks.push({
            ...meta(c),
            type: 'error',
            code: 'demo_unavailable',
            message: 'The demo provider is unavailable. Retry to continue.',
          })
          c.status = 'error'
          c.stream = null
          continue
        }
        const block = c.blocks.find((b) => b.id === stream.blockId)
        if (block?.type === 'text') block.text += stream.remaining.slice(0, 9)
        stream.remaining = stream.remaining.slice(9)
        if (stream.remaining) continue
        c.stream = null
        c.status = 'done'
        c.updatedAt = new Date().toISOString()
        const queued = c.queue.shift()
        if (queued) {
          c.blocks.push({
            ...meta(c),
            type: 'user',
            turnId: crypto.randomUUID(),
            content: queued.content,
            preamble: null,
          })
          this.start(c)
        }
      }
    },
    removeQueued(c: Conversation, id: string) {
      c.queue = c.queue.filter((item) => item.id !== id)
    },
    sendQueued(c: Conversation, id: string) {
      const item = c.queue.find((item) => item.id === id)
      if (!item || c.archived) return
      this.removeQueued(c, id)
      if (c.stream) {
        c.blocks.push({
          ...meta(c),
          type: 'steer',
          turnId: c.stream.blockId,
          content: item.content,
        })
      } else {
        c.blocks.push({
          ...meta(c),
          type: 'user',
          turnId: crypto.randomUUID(),
          content: item.content,
          preamble: null,
        })
        this.start(c)
      }
    },
    stop(c: Conversation) {
      if (!c.stream) return
      c.stream = null
      c.status = 'aborted'
      c.blocks.push({ ...meta(c), type: 'abort', isResumed: false })
    },
    compact(c: Conversation) {
      if (c.stream || !c.blocks.length || c.archived) return
      c.blocks.push({
        ...meta(c),
        type: 'compaction_boundary',
        summary:
          'The conversation’s goals and decisions are retained for the next turn. This is a simulated context summary.',
        summaryTokens: 28,
      })
      this.notice = 'Context compacted in this prototype.'
    },
  },
})
