import type { Block, SessionPhase, UserContentBlock } from '@demi/core'
import { applyTranscriptPatches } from './patch'
import type { ClientFrame, ClientSessionEvent, ProviderConfig, ServerFrame } from './frames'
import type { RpcClientTransport } from './transport'

export type RpcClientListener = (event: ClientSessionEvent) => void

type ActionCommand = 'send' | 'retry' | 'resume' | 'compact'

interface ActionWaiter {
  command: ActionCommand
  sawActivePhase: boolean
  resolve: () => void
  reject: (error: Error) => void
}

export class RpcClient {
  private readonly transport: RpcClientTransport
  private readonly listeners = new Set<RpcClientListener>()
  private readonly pendingActionWaiters: ActionWaiter[] = []
  private blocks: Block[] = []
  private phase: SessionPhase | null = null
  private unsubscribeTransport: () => void

  constructor(transport: RpcClientTransport) {
    this.transport = transport
    this.unsubscribeTransport = transport.onFrame((frame) => this.handleServerFrame(frame))
  }

  open(definition: string, provider: ProviderConfig, cwd: string): Promise<void> {
    const wait = this.waitForFrame('opened')
    this.sendFrame({ type: 'open', definition, provider, cwd })
    return wait
  }

  sendMessage(content: UserContentBlock[]): Promise<void> {
    const wait = this.waitForAction('send')
    this.sendFrame({ type: 'send', content })
    return wait
  }

  send(content: UserContentBlock[]): Promise<void> {
    return this.sendMessage(content)
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

  abort(): Promise<boolean> {
    if (this.phase === 'idle' && this.pendingActionWaiters.length === 0) return Promise.resolve(false)
    const wait = this.waitForAbort()
    this.sendFrame({ type: 'abort' })
    return wait
  }

  shellInput(sessionId: string, stdin: string): Promise<void> {
    const wait = this.waitForShellInput(sessionId)
    this.sendFrame({ type: 'shell_input', sessionId, stdin })
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

  subscribe(listener: RpcClientListener): () => void {
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
        this.emit({ type: 'transcript_snapshot', blocks: this.blocks })
        return
      case 'transcript_patch':
        this.blocks = applyTranscriptPatches(this.blocks, frame.patches)
        this.emit({ type: 'transcript_patch', patches: frame.patches, blocks: this.blocks })
        return
      case 'closed':
        this.blocks = []
        this.phase = null
        this.emit(frame)
        this.resolveAllActionWaiters()
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
      case 'tool_progress':
      case 'shell_output':
      case 'shell_input_result':
      case 'audit':
        this.emit(frame)
        return
      case 'rejected':
        this.emit(frame)
        this.rejectPendingAction(frame.command, new Error(frame.reason))
        return
      case 'error':
        this.emit(frame)
        this.rejectErroredAction(new Error(frame.message))
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

  private waitForAction(command: ActionCommand): Promise<void> {
    return new Promise((resolve, reject) => {
      this.pendingActionWaiters.push({
        command,
        sawActivePhase: false,
        resolve,
        reject,
      })
    })
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

  private settleActionWaiter(waiter: ActionWaiter, settle: () => void): void {
    const index = this.pendingActionWaiters.indexOf(waiter)
    if (index === -1) return
    this.pendingActionWaiters.splice(index, 1)
    settle()
  }

  private waitForAbort(): Promise<boolean> {
    return new Promise((resolve, reject) => {
      const unsubscribe = this.subscribe((event) => {
        if (event.type === 'phase' && event.phase === 'idle') {
          unsubscribe()
          resolve(true)
          return
        }
        if (event.type === 'closed') {
          unsubscribe()
          resolve(true)
          return
        }
        if (event.type === 'error') {
          unsubscribe()
          reject(new Error(event.message))
          return
        }
        if (event.type === 'rejected' && event.command === 'abort') {
          unsubscribe()
          reject(new Error(event.reason))
        }
      })
    })
  }

  private waitForShellInput(sessionId: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const unsubscribe = this.subscribe((event) => {
        if (event.type === 'shell_output' && event.sessionId === sessionId) {
          unsubscribe()
          resolve()
          return
        }
        if (event.type === 'shell_input_result' && event.sessionId === sessionId) {
          unsubscribe()
          resolve()
          return
        }
        if (event.type === 'error') {
          unsubscribe()
          reject(new Error(event.message))
          return
        }
        if (event.type === 'rejected' && event.command === 'shell_input') {
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
}
