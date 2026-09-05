import type { SessionNode } from '../node/node'
import type { AgentSession } from '../session/session'
import type { ChildJob, ChildSupervisor } from './supervisor'
import type { AgentTreeNode } from './format'

/**
 * Root-session-scoped flat registry of every live session in the tree: the root
 * plus every subagent at any depth. The sole basis for cross-tree addressing —
 * `send`, `steer`, `show`, and `list` resolve here, with no routing rules
 * along the tree.
 */
export class AgentDirectory<State = unknown> {
  private root: SessionNode<State> | null = null
  private readonly entries = new Map<string, { job: ChildJob<State>; owner: ChildSupervisor<State> }>()

  attachRoot(node: SessionNode<State>): void {
    this.root = node
  }

  rootId(): string {
    if (!this.root) throw new Error('agent directory has no root session')
    return this.root.id
  }

  rootSession(): AgentSession<State> {
    if (!this.root) throw new Error('agent directory has no root session')
    return this.root.session
  }

  register(job: ChildJob<State>, owner: ChildSupervisor<State>): void {
    this.entries.set(job.id, { job, owner })
  }

  unregister(id: string): void {
    this.entries.delete(id)
  }

  liveEntry(id: string): { job: ChildJob<State>; owner: ChildSupervisor<State> } | null {
    return this.entries.get(id) ?? null
  }

  /** The parent session id of a live agent; null for the root, undefined for an unknown id. */
  parentIdOf(id: string): string | null | undefined {
    if (this.root && this.root.id === id) return null
    return this.entries.get(id)?.owner.ownerId()
  }

  /**
   * The whole session tree: the root, every live agent, and each live node's
   * archived children (their supervisors exist, so their archives are
   * readable). Live children order by spawn time; archived newest first.
   */
  async tree(): Promise<AgentTreeNode[]> {
    if (!this.root) return []
    const build = async (
      id: string,
      parentId: string | null,
      job: ChildJob<State> | null,
      owner: ChildSupervisor<State> | null,
      supervisor: ChildSupervisor<State>,
    ): Promise<AgentTreeNode> => {
      const liveChildren = [...this.entries.values()]
        .filter((entry) => entry.owner === supervisor)
        .sort((a, b) => a.job.spawnedAt - b.job.spawnedAt)
      const children: AgentTreeNode[] = []
      for (const entry of liveChildren) {
        children.push(await build(entry.job.id, id, entry.job, entry.owner, entry.job.node.supervisor))
      }
      const now = Date.now()
      for (const archived of await supervisor.listArchivedJobs()) {
        children.push({
          id: archived.id,
          parentId: id,
          kind: 'archived',
          description: archived.description,
          profile: archived.profileName,
          phase: archived.closedPhase ?? 'completed',
          closedAgoMs: archived.closedAt === null ? null : now - archived.closedAt,
          line: null,
          children: [],
        })
      }
      if (job && owner) {
        return {
          id,
          parentId,
          kind: 'live',
          description: job.description,
          profile: job.profileName,
          phase: job.phase,
          closedAgoMs: null,
          line: owner.renderListLine(job),
          children,
        }
      }
      return {
        id,
        parentId,
        kind: 'root',
        description: '',
        profile: null,
        phase: 'running',
        closedAgoMs: null,
        line: null,
        children,
      }
    }
    return [await build(this.root.id, null, null, null, this.root.supervisor)]
  }
}
