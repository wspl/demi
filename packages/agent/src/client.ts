import type { Block, SessionPhase, UserContentBlock } from '@demicodes/core'
import { applyTranscriptPatches } from './patch'
import type { ProviderSelection } from '@demicodes/provider'
import type { ClientFrame, ClientSessionEvent, ConversationSummary, ServerFrame } from './frames'
import type { AgentClientTransport } from './transport'
import type { AbortResult } from './types'

export type AgentClientListener = (event: ClientSessionEvent) => void

type ActionCommand = 'send' | 'retry' | 'resume' | 'compact'

interface ActionWaiter {
  command: ActionCommand
  messageId?: string
  sawActivePhase: boolean
  resolve: () => void
  reject: (error: Error) => void
}

interface SteerWaiter {
  resolve: () => void
  reject: (error: Error) => void
}

interface AbortWaiter {
  resolve: (result: AbortResult) => void
  reject: (error: Error) => void
}

export class AgentClient {
  private readonly transport: AgentClientTransport
  private readonly listeners = new Set<AgentClientListener>()
  private readonly pendingActionWaiters: ActionWaiter[] = []
  private readonly pendingSteerWaiters = new Map<string, SteerWaiter>()
  private readonly pendingAbortWaiters: AbortWaiter[] = []
  private readonly pendingConversationWaiters: ((conversations: ConversationSummary[]) => void)[] = []
  private readonly queuedMessageIds = new Set<string>()
  private blocks: Block[] = []
  private revision: number | null = null
  private awaitingResync = false
  private phase: SessionPhase | null = null
  private unsubscribeTransport: () => void

  constructor(transport: AgentClientTransport) {
    this.transport = transport
    this.unsubscribeTransport = transport.onFrame((frame) => this.handleServerFrame(frame))
  }

  // sessionId is the conversation's stable, caller-owned id: reconnecting with
  // the same id resumes that conversation. Required so a session is never
  // silently un-resumable.
  open(provider: ProviderSelection, cwd: string, sessionId: string): Promise<void> {
    const wait = this.waitForFrame('opened')
    this.sendFrame({ type: 'open', provider, cwd, sessionId })
    return wait
  }

  // List the persisted conversations for a workspace (cwd) from the server,
  // newest first. Does not require an open session.
  listConversations(cwd: string): Promise<ConversationSummary[]> {
    return new Promise((resolve) => {
      this.pendingConversationWaiters.push(resolve)
      this.sendFrame({ type: 'list_conversations', cwd })
    })
  }

  sendMessage(content: UserContentBlock[]): Promise<void> {
    const messageId = globalThis.crypto.randomUUID()
    const wait = this.waitForAction('send', messageId)
    this.sendFrame({ type: 'send', messageId, content })
    return wait
  }

  send(content: UserContentBlock[]): Promise<void> {
    return this.sendMessage(content)
  }

  dequeueMessage(messageId: string): void {
    this.sendFrame({ type: 'dequeue_message', messageId })
    this.resolveQueuedSendWaiter(messageId)
    this.queuedMessageIds.delete(messageId)
  }

  sendQueuedMessage(messageId: string): void {
    this.moveQueuedSendWaiterToFront(messageId)
    this.sendFrame({ type: 'send_queued_message', messageId })
  }

  steerQueuedMessage(messageId: string, options: { steerId?: string } = {}): Promise<void> {
    const steerId = options.steerId ?? globalThis.crypto.randomUUID()
    const wait = this.waitForSteer(steerId)
    this.sendFrame({ type: 'steer_queued_message', messageId, steerId })
    return wait.then(() => {
      this.resolveQueuedSendWaiter(messageId)
      this.queuedMessageIds.delete(messageId)
    })
  }

  clearMessageQueue(messageIds: string[] = [...this.queuedMessageIds]): void {
    this.sendFrame({ type: 'clear_message_queue' })
    for (const messageId of messageIds) {
      this.resolveQueuedSendWaiter(messageId)
      this.queuedMessageIds.delete(messageId)
    }
  }

  steer(content: UserContentBlock[], options: { steerId?: string } = {}): Promise<void> {
    const steerId = options.steerId ?? globalThis.crypto.randomUUID()
    const wait = this.waitForSteer(steerId)
    this.sendFrame({ type: 'steer', steerId, content })
    return wait
  }

  cancelPendingSteer(steerId: string): void {
    this.sendFrame({ type: 'cancel_pending_steer', steerId })
  }

  /**
   * Switches the provider/model for an open session. The change takes effect on the next
   * turn (the server applies it at a turn boundary), so this is fire-and-forget.
   */
  setProvider(provider: ProviderSelection): void {
    this.sendFrame({ type: 'set_provider', provider })
  }

  retry(): Promise<void> {
    const wait = this.waitForAction('retry')
    this.sendFrame({ type: 'retry' })
    return wait
  }

  resume(): Promise<void> {
    const wait = this.waitForAction('resume')
    this.sendFrame({ type: 'resume' })
    return wait
  }

  compact(): Promise<void> {
    const wait = this.waitForAction('compact')
    this.sendFrame({ type: 'compact' })
    return wait
  }

  abort(): Promise<AbortResult> {
    const wait = this.waitForAbort()
    this.sendFrame({ type: 'abort' })
    return wait
  }

  shellWrite(commandId: string, stdin: string): Promise<void> {
    const wait = this.waitForShellWrite(commandId)
    this.sendFrame({ type: 'shell_write', commandId, stdin })
    return wait
  }

  close(): Promise<void> {
    const wait = this.waitForFrame('closed')
    this.sendFrame({ type: 'close' })
    return wait.finally(() => {
      this.unsubscribeTransport()
      this.transport.close()
    })
  }

  subscribe(listener: AgentClientListener): () => void {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  transcript(): { blocks: Block[] } {
    return { blocks: [...this.blocks] }
  }

  private sendFrame(frame: ClientFrame): void {
    this.transport.send(frame)
  }

  private handleServerFrame(frame: ServerFrame): void {
    switch (frame.type) {
      case 'transcript_snapshot':
        this.blocks = [...frame.blocks]
        this.revision = frame.revision
        this.awaitingResync = false
        this.emit({ type: 'transcript_snapshot', blocks: this.blocks })
        return
      case 'transcript_patch':
        if (this.awaitingResync) return
        // Transports are ordered, so a gap means a dropped frame somewhere in
        // the pipeline — fall back to a full snapshot instead of diverging.
        if (this.revision !== null && frame.revision !== this.revision + 1) {
          this.awaitingResync = true
          this.sendFrame({ type: 'sync_transcript' })
          return
        }
        this.revision = frame.revision
        this.blocks = applyTranscriptPatches(this.blocks, frame.patches)
        this.emit({ type: 'transcript_patch', patches: frame.patches, blocks: this.blocks })
        return
      case 'closed':
        this.blocks = []
        this.revision = null
        this.awaitingResync = false
        this.phase = null
        this.queuedMessageIds.clear()
        this.emit(frame)
        this.resolveAllActionWaiters()
        this.rejectAllSteerWaiters(new Error('Session closed'))
        this.rejectAllAbortWaiters(new Error('Session closed'))
        return
      case 'opened':
        this.emit(frame)
        return
      case 'phase': {
        const previousPhase = this.phase
        this.phase = frame.phase
        this.emit(frame)
        this.handleActionPhase(previousPhase, frame.phase)
        return
      }
      case 'queue':
        this.queuedMessageIds.clear()
        for (const message of frame.queue) this.queuedMessageIds.add(message.id)
        this.emit(frame)
        return
      case 'steer_result':
        this.emit(frame)
        this.handleSteerResult(frame)
        return
      case 'abort_result':
        this.emit(frame)
        this.resolveAbortWaiter(frame.result)
        return
      case 'tool_progress':
      case 'shell_output':
      case 'shell_write_result':
      case 'audit':
      case 'retry_scheduled':
        this.emit(frame)
        return
      case 'conversations':
        this.pendingConversationWaiters.shift()?.(frame.conversations)
        return
      case 'rejected':
        this.emit(frame)
        this.rejectPendingAction(frame.command, new Error(frame.reason))
        if (frame.command === 'abort') this.rejectAllAbortWaiters(new Error(frame.reason))
        return
      case 'error':
        this.emit(frame)
        this.rejectErroredAction(new Error(frame.message))
        this.rejectAllSteerWaiters(new Error(frame.message))
        this.rejectAllAbortWaiters(new Error(frame.message))
        return
    }
  }

  private waitForFrame<T extends ClientSessionEvent['type']>(type: T): Promise<void> {
    return new Promise((resolve, reject) => {
      const unsubscribe = this.subscribe((event) => {
        if (event.type === type) {
          unsubscribe()
          resolve()
        } else if (event.type === 'error') {
          unsubscribe()
          reject(new Error(event.message))
        } else if (event.type === 'rejected') {
          unsubscribe()
          reject(new Error(event.reason))
        }
      })
    })
  }

  private waitForAction(command: ActionCommand, messageId?: string): Promise<void> {
    return new Promise((resolve, reject) => {
      this.pendingActionWaiters.push({
        command,
        messageId,
        sawActivePhase: false,
        resolve,
        reject,
      })
    })
  }

  private waitForSteer(steerId: string): Promise<void> {
    return new Promise((resolve, reject) => {
      this.pendingSteerWaiters.set(steerId, { resolve, reject })
    })
  }

  private handleSteerResult(frame: Extract<ServerFrame, { type: 'steer_result' }>): void {
    const waiter = this.pendingSteerWaiters.get(frame.steerId)
    if (!waiter) return
    this.pendingSteerWaiters.delete(frame.steerId)
    if (frame.status === 'accepted') waiter.resolve()
    else waiter.reject(new Error(frame.reason))
  }

  private handleActionPhase(previousPhase: SessionPhase | null, phase: SessionPhase): void {
    if (phase !== 'idle') {
      if (previousPhase === 'idle' || previousPhase === null) this.markNextActionActive()
      return
    }
    this.resolveActiveAction()
  }

  private markNextActionActive(): void {
    const waiter = this.pendingActionWaiters.find((candidate) => !candidate.sawActivePhase)
    if (waiter) waiter.sawActivePhase = true
  }

  private resolveActiveAction(): void {
    const waiter = this.pendingActionWaiters.find((candidate) => candidate.sawActivePhase)
    if (!waiter) return
    this.settleActionWaiter(waiter, () => waiter.resolve())
  }

  private resolveQueuedSendWaiter(messageId: string): void {
    const waiter = this.pendingActionWaiters.find(
      (candidate) => candidate.command === 'send' && candidate.messageId === messageId && !candidate.sawActivePhase,
    )
    if (!waiter) return
    this.settleActionWaiter(waiter, () => waiter.resolve())
  }

  private moveQueuedSendWaiterToFront(messageId: string): void {
    const waiter = this.pendingActionWaiters.find(
      (candidate) => candidate.command === 'send' && candidate.messageId === messageId && !candidate.sawActivePhase,
    )
    if (!waiter) return
    const currentIndex = this.pendingActionWaiters.indexOf(waiter)
    if (currentIndex === -1) return
    this.pendingActionWaiters.splice(currentIndex, 1)

    const insertionIndex = this.pendingActionWaiters.findIndex(
      (candidate) => candidate.command === 'send' && !candidate.sawActivePhase,
    )
    if (insertionIndex === -1) this.pendingActionWaiters.push(waiter)
    else this.pendingActionWaiters.splice(insertionIndex, 0, waiter)
  }

  private rejectPendingAction(command: string, error: Error): void {
    const waiter =
      this.pendingActionWaiters.find((candidate) => candidate.command === command && !candidate.sawActivePhase) ??
      this.pendingActionWaiters.find((candidate) => candidate.command === command)
    if (!waiter) return
    this.settleActionWaiter(waiter, () => waiter.reject(error))
  }

  private rejectErroredAction(error: Error): void {
    const waiter = this.pendingActionWaiters.find((candidate) => candidate.sawActivePhase)
    if (waiter) {
      this.settleActionWaiter(waiter, () => waiter.reject(error))
      return
    }
    this.rejectAllActionWaiters(error)
  }

  private resolveAllActionWaiters(): void {
    const waiters = this.pendingActionWaiters.splice(0)
    for (const waiter of waiters) waiter.resolve()
  }

  private rejectAllActionWaiters(error: Error): void {
    const waiters = this.pendingActionWaiters.splice(0)
    for (const waiter of waiters) waiter.reject(error)
  }

  private rejectAllSteerWaiters(error: Error): void {
    const waiters = [...this.pendingSteerWaiters.values()]
    this.pendingSteerWaiters.clear()
    for (const waiter of waiters) waiter.reject(error)
  }

  private settleActionWaiter(waiter: ActionWaiter, settle: () => void): void {
    const index = this.pendingActionWaiters.indexOf(waiter)
    if (index === -1) return
    this.pendingActionWaiters.splice(index, 1)
    settle()
  }

  private waitForAbort(): Promise<AbortResult> {
    return new Promise((resolve, reject) => {
      this.pendingAbortWaiters.push({ resolve, reject })
    })
  }

  private waitForShellWrite(commandId: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const unsubscribe = this.subscribe((event) => {
        if (event.type === 'shell_output' && event.commandId === commandId) {
          unsubscribe()
          resolve()
          return
        }
        if (event.type === 'shell_write_result' && event.commandId === commandId) {
          unsubscribe()
          resolve()
          return
        }
        if (event.type === 'error') {
          unsubscribe()
          reject(new Error(event.message))
          return
        }
        if (event.type === 'rejected' && event.command === 'shell_write') {
          unsubscribe()
          reject(new Error(event.reason))
          return
        }
        if (event.type === 'closed') {
          unsubscribe()
          resolve()
        }
      })
    })
  }

  private emit(event: ClientSessionEvent): void {
    for (const listener of this.listeners) listener(event)
  }

  private resolveAbortWaiter(result: AbortResult): void {
    const waiter = this.pendingAbortWaiters.shift()
    if (!waiter) return
    waiter.resolve(result)
  }

  private rejectAllAbortWaiters(error: Error): void {
    const waiters = this.pendingAbortWaiters.splice(0)
    for (const waiter of waiters) waiter.reject(error)
  }
}
