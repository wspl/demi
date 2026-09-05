import type { LiveSession } from './live-session'

/** What the registry needs from an attached party: the ability to be detached on takeover. */
export interface SessionAttachment {
  handleTakeover(): Promise<void>
}

/**
 * Owns every live session in this server and tracks which transport binding is
 * currently attached to each. Sessions live in the server, not in bindings: a
 * binding going away merely detaches its subscription, and opening a session
 * id that another binding is attached to takes the attachment over — the
 * running session object itself is adopted, never restarted.
 */
export class SessionOwnershipRegistry {
  private readonly live = new Map<string, LiveSession>()
  private readonly attached = new Map<string, SessionAttachment>()

  /** Detaches any other binding from the id and records this one; returns the live session to adopt. */
  async claim(sessionId: string, binding: SessionAttachment): Promise<LiveSession | null> {
    const previous = this.attached.get(sessionId)
    if (previous && previous !== binding) await previous.handleTakeover()
    this.attached.set(sessionId, binding)
    return this.live.get(sessionId) ?? null
  }

  register(live: LiveSession): void {
    this.live.set(live.agentSessionId, live)
  }

  unregister(sessionId: string): void {
    this.live.delete(sessionId)
  }

  release(sessionId: string, binding: SessionAttachment): void {
    if (this.attached.get(sessionId) === binding) this.attached.delete(sessionId)
  }

  get(sessionId: string): LiveSession | null {
    return this.live.get(sessionId) ?? null
  }

  sessions(): LiveSession[] {
    return [...this.live.values()]
  }

  async disposeAll(): Promise<void> {
    const sessions = this.sessions()
    this.live.clear()
    this.attached.clear()
    await Promise.all(sessions.map((live) => live.dispose()))
  }
}
