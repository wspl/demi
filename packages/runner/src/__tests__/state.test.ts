import { mkdtemp, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { RunnerState } from '../state'

test('runner state persists config and token (token file 0600)', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-runner-state-'))
  const state = new RunnerState(dir)

  expect(await state.readConfig()).toBeNull()
  expect(await state.readToken()).toBeNull()

  await state.writeConfig({ backendUrl: 'wss://backend.example', deviceId: 'dev-1' })
  expect(await state.readConfig()).toEqual({ backendUrl: 'wss://backend.example', deviceId: 'dev-1' })

  await state.writeToken('secret-token')
  expect(await state.readToken()).toBe('secret-token')
  const mode = (await stat(join(dir, 'runner-token'))).mode & 0o777
  expect(mode).toBe(0o600)
})
