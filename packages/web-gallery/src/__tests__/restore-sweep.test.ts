import { expect, test } from 'bun:test'
import { delay } from '@demicodes/utils'
import { RestoreSweep } from '../fixtures/restore-sweep'

test('retry sweeps loading then ready', async () => {
  const sweep = new RestoreSweep()
  const phases: string[] = []
  sweep.start((phase) => phases.push(phase), 20)
  expect(phases).toEqual(['loading'])
  await delay(40)
  expect(phases).toEqual(['loading', 'ready'])
})

test('a second retry cancels the first', async () => {
  const sweep = new RestoreSweep()
  const phases: string[] = []
  sweep.start((phase) => phases.push(`a:${phase}`), 40)
  await delay(10)
  sweep.start((phase) => phases.push(`b:${phase}`), 20)
  await delay(50)
  expect(phases).toEqual(['a:loading', 'b:loading', 'b:ready'])
})

test('stop prevents ready', async () => {
  const sweep = new RestoreSweep()
  const phases: string[] = []
  sweep.start((phase) => phases.push(phase), 20)
  sweep.stop()
  await delay(40)
  expect(phases).toEqual(['loading'])
})
