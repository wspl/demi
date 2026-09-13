import { describe, expect, it } from 'bun:test'
import { buildScrollbarMarkers, resolveScrollbarMarkers } from '../scrollbars/markers'

describe('resolveScrollbarMarkers', () => {
  it('keeps overlapping markers as layered ranges ordered by priority', () => {
    const markers = resolveScrollbarMarkers([
      { kind: 'diff-added', startRatio: 0.1, endRatio: 0.2, priority: 1 },
      { kind: 'diagnostic', startRatio: 0.15, endRatio: 0.25, priority: 2 },
      { kind: 'search', startRatio: 0.18, endRatio: 0.22, priority: 3 },
    ])

    expect(markers).toEqual([
      { kind: 'diff-added', startRatio: 0.1, endRatio: 0.15, priority: 1, severity: undefined },
      { kind: 'diagnostic', startRatio: 0.15, endRatio: 0.18, priority: 2, severity: undefined },
      { kind: 'diff-added', startRatio: 0.15, endRatio: 0.18, priority: 1, severity: undefined },
      { kind: 'search', startRatio: 0.18, endRatio: 0.2, priority: 3, severity: undefined },
      { kind: 'diagnostic', startRatio: 0.18, endRatio: 0.2, priority: 2, severity: undefined },
      { kind: 'diff-added', startRatio: 0.18, endRatio: 0.2, priority: 1, severity: undefined },
      { kind: 'search', startRatio: 0.2, endRatio: 0.22, priority: 3, severity: undefined },
      { kind: 'diagnostic', startRatio: 0.2, endRatio: 0.25, priority: 2, severity: undefined },
    ])
  })

  it('merges adjacent ranges of the same kind', () => {
    const markers = resolveScrollbarMarkers([
      { kind: 'diagnostic', startRatio: 0.4, endRatio: 0.41, severity: 'warning', priority: 2 },
      { kind: 'diagnostic', startRatio: 0.41, endRatio: 0.43, severity: 'warning', priority: 2 },
    ])

    expect(markers).toEqual([
      { kind: 'diagnostic', startRatio: 0.4, endRatio: 0.43, severity: 'warning', priority: 2 },
    ])
  })

  it('maps diagnostics to vertical scrollbar markers', () => {
    const markers = buildScrollbarMarkers({
      totalLines: 200,
      diagnostics: [{ fromLine: 10, toLine: 12, severity: 'warning' }],
    })

    expect(markers).toContainEqual(expect.objectContaining({
      kind: 'diagnostic',
      severity: 'warning',
    }))
  })

  it('prefers the most severe diagnostic when diagnostics overlap', () => {
    const markers = resolveScrollbarMarkers([
      { kind: 'diagnostic', startRatio: 0.1, endRatio: 0.2, severity: 'info', priority: 2 },
      { kind: 'diagnostic', startRatio: 0.1, endRatio: 0.2, severity: 'error', priority: 2 },
    ])

    expect(markers).toEqual(expect.arrayContaining([
      { kind: 'diagnostic', startRatio: 0.1, endRatio: 0.2, severity: 'info', priority: 2 },
      { kind: 'diagnostic', startRatio: 0.1, endRatio: 0.2, severity: 'error', priority: 2 },
    ]))
  })

  it('includes search and selection markers with explicit priorities', () => {
    const markers = buildScrollbarMarkers({
      totalLines: 100,
      searches: [{ fromLine: 20, toLine: 20 }],
      selections: [{ fromLine: 20, toLine: 20 }],
    })

    expect(markers).toEqual(expect.arrayContaining([
      { kind: 'selection', startRatio: 0.19, endRatio: 0.2, severity: undefined, priority: 4 },
      { kind: 'search', startRatio: 0.19, endRatio: 0.2, severity: undefined, priority: 3 },
    ]))
  })

  it('keeps added and deleted diff markers as separate layers', () => {
    const markers = buildScrollbarMarkers({
      totalLines: 100,
      diffs: [
        { fromLine: 30, toLine: 31, kind: 'added' },
        { fromLine: 30, toLine: 31, kind: 'deleted' },
      ],
    })

    expect(markers).toEqual(expect.arrayContaining([
      { kind: 'diff-added', startRatio: 0.29, endRatio: 0.31, severity: undefined, priority: 1 },
      { kind: 'diff-deleted', startRatio: 0.29, endRatio: 0.31, severity: undefined, priority: 1 },
    ]))
  })
})
