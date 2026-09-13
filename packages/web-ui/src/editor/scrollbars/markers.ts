export type ScrollbarMarkerKind = 'selection' | 'search' | 'diagnostic' | 'diff-added' | 'diff-deleted'
export type ScrollbarMarkerSeverity = 'error' | 'warning' | 'info' | 'hint'

export interface RawScrollbarMarker {
  kind: ScrollbarMarkerKind
  startRatio: number
  endRatio: number
  severity?: ScrollbarMarkerSeverity
  priority: number
}

export interface DiagnosticScrollbarInput {
  fromLine: number
  toLine: number
  severity: ScrollbarMarkerSeverity
}

export interface SearchScrollbarInput {
  fromLine: number
  toLine: number
}

export interface DiffScrollbarInput {
  fromLine: number
  toLine: number
  kind: 'added' | 'deleted'
  startRatio?: number
  endRatio?: number
}

export interface SelectionScrollbarInput {
  fromLine: number
  toLine: number
}

const priority: Record<ScrollbarMarkerKind, number> = {
  selection: 4,
  search: 3,
  diagnostic: 2,
  'diff-added': 1,
  'diff-deleted': 1,
}

const diagnosticSeverityPriority: Record<ScrollbarMarkerSeverity, number> = {
  error: 4,
  warning: 3,
  info: 2,
  hint: 1,
}

function compareMarkers(left: RawScrollbarMarker, right: RawScrollbarMarker) {
  const base = right.priority - left.priority
  if (base !== 0) return base
  if (left.kind === 'diagnostic' && right.kind === 'diagnostic') {
    return diagnosticSeverityPriority[right.severity ?? 'info'] - diagnosticSeverityPriority[left.severity ?? 'info']
  }
  return 0
}

export function resolveScrollbarMarkers(input: RawScrollbarMarker[]) {
  const edges = [...new Set(input.flatMap((marker) => [marker.startRatio, marker.endRatio]))]
    .sort((a, b) => a - b)
  const resolved: RawScrollbarMarker[] = []

  for (let index = 0; index < edges.length - 1; index++) {
    const startRatio = edges[index]!
    const endRatio = edges[index + 1]!
    const covering = input
      .filter((marker) => marker.startRatio < endRatio && marker.endRatio > startRatio)
      .sort(compareMarkers)

    if (!covering.length) continue
    for (const marker of covering) {
      const next: RawScrollbarMarker = {
        kind: marker.kind,
        startRatio,
        endRatio,
        severity: marker.severity,
        priority: marker.priority,
      }
      const previous = resolved.at(-1)
      if (
        previous
        && previous.kind === next.kind
        && previous.severity === next.severity
        && previous.priority === next.priority
        && previous.endRatio === next.startRatio
      ) {
        previous.endRatio = next.endRatio
      } else {
        resolved.push(next)
      }
    }
  }

  return resolved
}

export function buildScrollbarMarkers(input: {
  totalLines: number
  diagnostics?: DiagnosticScrollbarInput[]
  searches?: SearchScrollbarInput[]
  selections?: SelectionScrollbarInput[]
  diffs?: DiffScrollbarInput[]
}) {
  const totalLines = Math.max(input.totalLines, 1)
  const minimumSpan = 1 / totalLines
  const raw: RawScrollbarMarker[] = [
    ...(input.diagnostics ?? []).map((diagnostic) => ({
      kind: 'diagnostic' as const,
      startRatio: Math.max(0, (diagnostic.fromLine - 1) / totalLines),
      endRatio: Math.min(1, diagnostic.toLine / totalLines),
      severity: diagnostic.severity,
      priority: priority.diagnostic,
    })),
    ...(input.searches ?? []).map((match) => ({
      kind: 'search' as const,
      startRatio: Math.max(0, (match.fromLine - 1) / totalLines),
      endRatio: Math.min(1, Math.max(match.toLine / totalLines, (match.fromLine - 1) / totalLines + minimumSpan)),
      priority: priority.search,
    })),
    ...(input.diffs ?? []).map((diff) => ({
      kind: diff.kind === 'deleted' ? 'diff-deleted' as const : 'diff-added' as const,
      startRatio: diff.startRatio ?? Math.max(0, (diff.fromLine - 1) / totalLines),
      endRatio: diff.endRatio ?? Math.min(1, Math.max(diff.toLine / totalLines, (diff.fromLine - 1) / totalLines + minimumSpan)),
      priority: diff.kind === 'deleted' ? priority['diff-deleted'] : priority['diff-added'],
    })),
    ...(input.selections ?? []).map((selection) => ({
      kind: 'selection' as const,
      startRatio: Math.max(0, (selection.fromLine - 1) / totalLines),
      endRatio: Math.min(1, Math.max(selection.toLine / totalLines, (selection.fromLine - 1) / totalLines + minimumSpan)),
      priority: priority.selection,
    })),
  ]

  return resolveScrollbarMarkers(raw)
}
