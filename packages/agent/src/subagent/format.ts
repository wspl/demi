import type { SubagentJob } from '../protocol/frames'

export interface AgentTreeNode {
  id: string
  parentId: string | null
  kind: 'root' | 'live' | 'archived'
  description: string
  profile: string | null
  phase: SubagentJob['phase']
  closedAgoMs: number | null
  /** Pre-rendered live status line (id, phase, ages, execution, activity); null for root/archived. */
  line: string | null
  children: AgentTreeNode[]
}

/** Compact human duration for subagent roster/status lines. */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000))
  if (totalSeconds < 60) return `${totalSeconds}s`
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  if (minutes < 60) return seconds > 0 ? `${minutes}m${seconds}s` : `${minutes}m`
  const hours = Math.floor(minutes / 60)
  const remMinutes = minutes % 60
  return remMinutes > 0 ? `${hours}h${remMinutes}m` : `${hours}h`
}

export function renderTreeNode(node: AgentTreeNode, prefix: string, isLast: boolean, selfId: string, lines: string[]): void {
  const marker = node.id === selfId ? ' ← you' : ''
  const body =
    node.kind === 'root'
      ? `● ${node.id}  (root session)${marker}`
      : node.kind === 'archived'
        ? `○ ${node.id}  archived (${node.phase}${node.closedAgoMs === null ? '' : ` ${formatDuration(node.closedAgoMs)} ago`})  ${node.description ? `"${node.description}"` : '(no description)'}`
        : `● ${node.line}${marker}`
  if (node.parentId === null) lines.push(body)
  else lines.push(`${prefix}${isLast ? '└─' : '├─'}${body}`)
  const childPrefix = node.parentId === null ? '' : `${prefix}${isLast ? '  ' : '│ '}`
  node.children.forEach((child, index) => {
    renderTreeNode(child, childPrefix, index === node.children.length - 1, selfId, lines)
  })
}

export function flattenTree(nodes: AgentTreeNode[], selfId: string): Record<string, unknown>[] {
  const flat: Record<string, unknown>[] = []
  const visit = (node: AgentTreeNode): void => {
    flat.push({
      subagentId: node.id,
      parentSessionId: node.parentId,
      kind: node.kind,
      description: node.description,
      profile: node.profile,
      phase: node.phase,
      closedAgoMs: node.closedAgoMs,
      self: node.id === selfId,
    })
    node.children.forEach(visit)
  }
  nodes.forEach(visit)
  return flat
}
