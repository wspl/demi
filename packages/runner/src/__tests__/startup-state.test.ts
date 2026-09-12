import { expect, test } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile, mkdir, stat } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { runnerBackendUrlSchema, runnerStartupFromEnv } from '../startup'
import { RunnerState } from '../state'
import { nodeFileSystem } from '../testing/node-fs'

test('runner environment has explicit managed flag and bounded reconnect delay', () => {
  expect(runnerStartupFromEnv({})).toMatchObject({ managed: false })
  expect(runnerStartupFromEnv({ DEMI_RUNNER_MANAGED: '0' }).managed).toBe(false)
  expect(
    runnerStartupFromEnv({
      DEMI_RUNNER_MANAGED: '1',
      DEMI_RUNNER_RECONNECT_MS: '30',
    }),
  ).toMatchObject({ managed: true, reconnect: { initialDelayMs: 30 } })
  for (const value of ['', ' ', '0', '-1', '1.5', 'NaN', '2147483648', '1e3']) {
    expect(() =>
      runnerStartupFromEnv({ DEMI_RUNNER_RECONNECT_MS: value }),
    ).toThrow('DEMI_RUNNER_RECONNECT_MS')
  }
  for (const value of ['', 'false', 'true', '2']) {
    expect(() => runnerStartupFromEnv({ DEMI_RUNNER_MANAGED: value })).toThrow(
      'DEMI_RUNNER_MANAGED',
    )
  }
  expect(() => runnerStartupFromEnv({ DEMI_HOME: '' })).toThrow('DEMI_HOME')
  expect(() => runnerStartupFromEnv({ DEMI_RUNNER_NAME: ' ' })).toThrow(
    'DEMI_RUNNER_NAME',
  )
  expect(runnerBackendUrlSchema.safeParse('file:///backend').success).toBe(
    false,
  )
  expect(runnerBackendUrlSchema.parse('http://localhost:3271')).toBe(
    'http://localhost:3271',
  )
})

test('runner durable state separates absence, corruption and IO failures', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'demi-state-contract-'))
  const state = new RunnerState(nodeFileSystem(dir), dir)
  try {
    expect(await state.readToken()).toBeNull()
    expect(await state.readConfig()).toBeNull()
    await state.writeToken('synthetic-token')
    expect(await state.readToken()).toBe('synthetic-token')
    expect((await stat(state.tokenPath)).mode & 0o777).toBe(0o600)
    for (const value of ['', '\n  \t', 'two tokens']) {
      await writeFile(state.tokenPath, value)
      await expect(state.readToken()).rejects.toThrow('Invalid runner token')
      expect(await readFile(state.tokenPath, 'utf8')).toBe(value)
    }
    await expect(state.writeToken('')).rejects.toThrow('Invalid runner token')
    await rm(state.tokenPath)
    await mkdir(state.tokenPath)
    await expect(state.readToken()).rejects.toThrow()
    for (const value of [
      '{',
      'null',
      '{"backendUrl":"file:///backend"}',
      '{"backendUrl":"http://backend","deviceId":3}',
    ]) {
      await writeFile(state.configPath, value)
      await expect(state.readConfig()).rejects.toThrow()
      expect(await readFile(state.configPath, 'utf8')).toBe(value)
    }
    await state.writeConfig({
      backendUrl: 'https://backend.test',
      deviceId: 'device',
    })
    expect(await state.readConfig()).toEqual({
      backendUrl: 'https://backend.test',
      deviceId: 'device',
    })
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})
