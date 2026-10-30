/**
 * The steps of a server update as both of its surfaces show them: Settings ›
 * Updates while the server still serves, and the Updating Demi screen from
 * the restart on. Each surface names its own steps; the states and their
 * numbers are the same.
 */

/** The numbers of a running step. */
export type StepProgress =
  | { kind: 'bytes', done: number, total: number, perSecond?: number | null, leftSeconds?: number | null }
  | { kind: 'clouds', done: number, total: number }
  | { kind: 'conversations', done: number, total: number, bytesDone: number, bytes: number, leftSeconds?: number | null }

/** What a done step did. */
export type StepSummary =
  | { kind: 'none' }
  | { kind: 'bytes', bytes: number }
  | { kind: 'count', count: number }

/** How far one step is. */
export type StepState =
  | { state: 'waiting' }
  | { state: 'running', started: string, progress?: StepProgress | null }
  | { state: 'done', tookMs: number, summary: StepSummary }
  | { state: 'skipped', why: 'installed' | 'noChanges' }
  | { state: 'failed', reason: string }

/** One line of a step list, worded by its surface. */
export interface StepLine {
  key: string
  title: string
  mark: StepState['state']
  detail: string
}

/**
 * The share of the whole done, 0 to 1: each step counts by `weight`, the
 * time it usually takes, and a running step with numbers moves inside its
 * share.
 */
export function overallDone<S extends { state: StepState }>(steps: readonly S[], weight: (step: S) => number): number {
  let all = 0
  let done = 0
  for (const step of steps) {
    const share = weight(step)
    all += share
    const state = step.state
    if (state.state === 'done' || state.state === 'skipped')
      done += share
    else if (state.state === 'running' && state.progress)
      done += share * progressShare(state.progress)
  }
  return all === 0 ? 0 : done / all
}

function progressShare(progress: StepProgress): number {
  switch (progress.kind) {
    case 'bytes':
      return progress.total > 0 ? Math.min(1, progress.done / progress.total) : 0
    case 'conversations':
      return progress.bytes > 0 ? Math.min(1, progress.bytesDone / progress.bytes) : 0
    case 'clouds':
      return progress.total > 0 ? Math.min(1, progress.done / progress.total) : 0
  }
}

/** The step that runs, or the one that failed, with its place from 1. */
export function currentStep<S extends { state: StepState }>(steps: readonly S[]): { step: S, number: number } | null {
  const index = steps.findIndex((step) => step.state.state === 'running' || step.state.state === 'failed')
  return index === -1 ? null : { step: steps[index]!, number: index + 1 }
}
