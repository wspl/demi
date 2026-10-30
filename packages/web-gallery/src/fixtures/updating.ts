import type { RestartStep, RestartStepKind, UpdateProgress } from '@demicodes/web-ui/ui/UpdatingScreen.vue'
import type { StepState } from '@demicodes/web-ui/ui/update-steps'

const MB = 1024 * 1024
const KINDS: readonly RestartStepKind[] = ['stopDemi', 'saveClouds', 'keepDatabases', 'switch', 'startManager', 'startDemi', 'updateControl', 'updateConversations']

function done(kind: RestartStepKind): StepState {
  switch (kind) {
    case 'saveClouds':
      return { state: 'done', tookMs: 48_000, summary: { kind: 'count', count: 20 } }
    case 'keepDatabases':
      return { state: 'done', tookMs: 6_000, summary: { kind: 'bytes', bytes: 2048 * MB } }
    case 'updateControl':
      return { state: 'done', tookMs: 1_200, summary: { kind: 'none' } }
    default:
      return { state: 'done', tookMs: 1_800, summary: { kind: 'none' } }
  }
}

/**
 * The Updating Demi screen's progress with the steps before `kind` done, `kind` as `state`, and the rest waiting;
 * `kept` when the update keeps the running Clouds running rather than saving them.
 */
export function updatingAt(kind: RestartStepKind, state: StepState, returning = false, kept = false): UpdateProgress {
  const reached = KINDS.indexOf(kind)
  const steps = KINDS.map((each, index): RestartStep => ({
    kind: each,
    state: index < reached
      ? (each === 'saveClouds' && kept ? { state: 'done', tookMs: 900, summary: { kind: 'none' } } : done(each))
      : index === reached ? state : { state: 'waiting' },
    ...(each === 'saveClouds' && kept ? { kept: true } : {}),
  }))
  return { to: '0.1.5500', from: '0.1.5469', returning, steps }
}

const ago = (seconds: number) => new Date(Date.now() - seconds * 1000).toISOString()

/** The moments the gallery shows. */
export const updatingMoments = {
  saving: updatingAt('saveClouds', { state: 'running', started: ago(20), progress: { kind: 'clouds', done: 7, total: 20 } }),
  copying: updatingAt('keepDatabases', { state: 'running', started: ago(3), progress: { kind: 'bytes', done: 1229 * MB, total: 2048 * MB } }),
  keeping: updatingAt('startManager', { state: 'running', started: ago(2) }, false, true),
  conversations: updatingAt('updateConversations', { state: 'running', started: ago(70), progress: { kind: 'conversations', done: 41, total: 73, bytesDone: 2765 * MB, bytes: 6760 * MB, leftSeconds: 185 } }),
  returning: updatingAt('startDemi', { state: 'failed', reason: 'Demi 0.1.5500 did not start' }, true),
  failed: updatingAt('switch', { state: 'failed', reason: 'No space left on device (os error 28)' }),
  updated: {
    to: '0.1.5500',
    from: '0.1.5469',
    returning: false,
    steps: KINDS.map((kind): RestartStep => ({
      kind,
      state: kind === 'updateControl'
        ? { state: 'skipped', why: 'noChanges' }
        : kind === 'updateConversations'
          ? { state: 'done', tookMs: 130_000, summary: { kind: 'count', count: 73 } }
          : done(kind),
    })),
  },
} satisfies Record<string, UpdateProgress>
