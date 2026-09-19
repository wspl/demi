export type ScrollbarMarkerKind = 'selection' | 'search' | 'diff-added' | 'diff-deleted'

export interface RawScrollbarMarker {
  kind: ScrollbarMarkerKind
  startRatio: number
  endRatio: number
  priority: number
}

/** Lines from 1, inclusive. */
export interface LineRange {
  fromLine: number
  toLine: number
}

export interface DiffScrollbarInput extends LineRange {
  kind: 'added' | 'deleted'
}

const priority: Record<ScrollbarMarkerKind, number> = {
  selection: 3,
  search: 2,
  'diff-added': 1,
  'diff-deleted': 1,
}

function compareMarkers(left: RawScrollbarMarker, right: RawScrollbarMarker) {
  return right.priority - left.priority
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
        priority: marker.priority,
      }
      const previous = resolved.at(-1)
      if (
        previous
        && previous.kind === next.kind
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
  searches?: LineRange[]
  selections?: LineRange[]
  diffs?: DiffScrollbarInput[]
}) {
  const totalLines = Math.max(input.totalLines, 1)
  // A marker spans its lines, and at least one line's height.
  const marker = (kind: ScrollbarMarkerKind, range: LineRange): RawScrollbarMarker => {
    const startRatio = Math.max(0, (range.fromLine - 1) / totalLines)
    return {
      kind,
      startRatio,
      endRatio: Math.min(1, Math.max(range.toLine / totalLines, startRatio + 1 / totalLines)),
      priority: priority[kind],
    }
  }

  return resolveScrollbarMarkers([
    ...(input.searches ?? []).map((range) => marker('search', range)),
    ...(input.diffs ?? []).map((diff) => marker(diff.kind === 'deleted' ? 'diff-deleted' : 'diff-added', diff)),
    ...(input.selections ?? []).map((range) => marker('selection', range)),
  ])
}
