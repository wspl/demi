// What `demi.runner` does with the slot's runner (browse.md § What `demi`
// adds), decided from what the slot's state records of it and
// whether its process runs: the slot has at most one runner, paired once
// and then stopped and started again as a device that goes away and comes
// back, until `demi.runner({ fresh: true })` pairs a new one in its place.
import type { RunnerRecord } from './state'

export type RunnerRequest = 'default' | 'start' | 'stop' | 'new'

export type RunnerStep =
  | { kind: 'already running' }
  | { kind: 'not running' }
  /** Starts the paired runner again, with its installation as it was. */
  | { kind: 'start' }
  | { kind: 'stop' }
  /** Pairs a runner of `generation` in a new installation, stopping the running one first when `stop`. */
  | { kind: 'pair', generation: number, stop: boolean }
  /** `runner start` with no paired runner to start. */
  | { kind: 'nothing paired' }

/** The step `request` takes, given the runner `record` names and whether its process `runs`. */
export function runnerStep(request: RunnerRequest, record: RunnerRecord | undefined, runs: boolean): RunnerStep {
  const paired = record !== undefined && record.device !== null
  switch (request) {
    case 'stop':
      return runs ? { kind: 'stop' } : { kind: 'not running' }
    case 'start':
      if (!paired) {
        return { kind: 'nothing paired' }
      }
      return runs ? { kind: 'already running' } : { kind: 'start' }
    case 'default':
      if (runs && paired) {
        return { kind: 'already running' }
      }
      if (paired) {
        return { kind: 'start' }
      }
      // A pairing that did not end, such as one whose Add Device failed, is
      // started over in the same generation, its runner stopped first when
      // it still waits to be paired.
      return { kind: 'pair', generation: record?.generation ?? 1, stop: runs }
    case 'new':
      return { kind: 'pair', generation: (record?.generation ?? 0) + 1, stop: runs }
  }
}

/** The name the slot's runner of `generation` asks to be paired as. */
export function runnerName(slot: number, generation: number): string {
  const base = `browse-slot-${slot}`
  return generation === 1 ? base : `${base}-${generation}`
}
