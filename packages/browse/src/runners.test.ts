import { expect, test } from 'bun:test'
import { runnerName, runnerStep, type RunnerRequest } from './runners'
import type { RunnerRecord } from './state'

const paired: RunnerRecord = { generation: 1, device: 'browse-slot-2' }
const pairing: RunnerRecord = { generation: 2, device: null }

test('a paired runner is stopped and started again as the same device; only --new pairs another', () => {
  const cases: [RunnerRequest, RunnerRecord | undefined, boolean, ReturnType<typeof runnerStep>][] = [
    // The first runner of a slot is paired.
    ['default', undefined, false, { kind: 'pair', generation: 1, stop: false }],
    ['default', paired, true, { kind: 'already running' }],
    ['stop', paired, true, { kind: 'stop' }],
    ['stop', paired, false, { kind: 'not running' }],
    // A stopped paired runner comes back as the same device, by either command.
    ['start', paired, false, { kind: 'start' }],
    ['default', paired, false, { kind: 'start' }],
    ['start', paired, true, { kind: 'already running' }],
    ['start', undefined, false, { kind: 'nothing paired' }],
    // A pairing that never ended is started over, not counted as a new runner.
    ['default', pairing, false, { kind: 'pair', generation: 2, stop: false }],
    ['start', pairing, false, { kind: 'nothing paired' }],
    // --new stops the running runner and pairs the next generation.
    ['new', paired, true, { kind: 'pair', generation: 2, stop: true }],
    ['new', undefined, false, { kind: 'pair', generation: 1, stop: false }],
  ]
  for (const [request, record, runs, step] of cases) {
    expect([request, record, runs, runnerStep(request, record, runs)]).toEqual([request, record, runs, step])
  }
})

test('each generation of a slot\'s runner asks for a name of its own', () => {
  expect([1, 2, 3].map((generation) => runnerName(4, generation))).toEqual(['browse-slot-4', 'browse-slot-4-2', 'browse-slot-4-3'])
})
