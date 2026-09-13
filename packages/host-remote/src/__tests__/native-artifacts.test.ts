import { expect, test } from 'bun:test'
import { buildManifest } from '@demicodes/command-loader'
import { nativePackageSchema } from '@demicodes/command-protocol'
import type { BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { memoryHostStore } from '@demicodes/shell/testing'
import fixture from '../../../command-protocol/tests/fixtures/package.json'
import { RemoteHost } from '../remote-host'

const descriptor = nativePackageSchema.parse(fixture.descriptor)

async function setup(resolveArtifact: Parameters<RemoteHost['startJob']>[0]['commands']) {
  const host = new RemoteHost({ defaultCwd: '/work', identity: { uid: 1, gid: 1, hostname: 'fixture', homeDir: '/work' }, store: memoryHostStore() })
  const messages: BackendToRunnerMessage[] = []
  host.attach(message => messages.push(message))
  host.startJob({ script: 'native', cwd: '/work', env: {}, commands: resolveArtifact })
  const start = messages.find(message => message.type === 'job_start')!
  if (start.type !== 'job_start')
    throw new Error('Missing job start')
  return { host, messages, start }
}

async function manifest() {
  return buildManifest([{ name: 'native', summary: '', kind: 'native', binding: { package: descriptor.id, operation: descriptor.operations[0]! } }], { packages: [descriptor] })
}

test('artifact requests require the active job and its exact manifest artifact', async () => {
  const catalog = await manifest()
  let calls = 0
  const { host, messages, start } = await setup({ manifest: catalog, resolveArtifact: async () => {
    calls += 1
    return { url: 'https://artifacts.example.test/exact' }
  } })
  expect(messages[0]?.type).toBe('manifest')
  expect(start.manifestHash).toBe(catalog.hash)
  const request = { type: 'artifact_resolve' as const, id: 'request', jobId: start.jobId, manifestHash: catalog.hash, target: 'aarch64-apple-darwin' as const, sha256: descriptor.targets['aarch64-apple-darwin'].sha256 }
  host.handleMessage({ ...request, sha256: 'f'.repeat(64) })
  await Bun.sleep(0)
  expect(calls).toBe(0)
  expect(messages.at(-1)).toMatchObject({ type: 'artifact_location', error: 'Artifact does not belong to the active job catalog' })
  host.handleMessage(request)
  await Bun.sleep(0)
  expect(calls).toBe(1)
  expect(messages.at(-1)).toMatchObject({ type: 'artifact_location', location: { url: 'https://artifacts.example.test/exact' } })
  host.handleMessage({ type: 'job_exit', jobId: start.jobId, exitCode: 0 })
  host.handleMessage(request)
  expect(calls).toBe(1)
  expect(messages.at(-1)).toMatchObject({ error: 'No matching active job command catalog' })
  host.detach()
})

test('job completion aborts pending location resolution without a stale response', async () => {
  const catalog = await manifest()
  let observed: AbortSignal | undefined
  const { host, messages, start } = await setup({ manifest: catalog, resolveArtifact: async (_, signal) => {
    observed = signal
    await new Promise((_, reject) => signal.addEventListener('abort', () => reject(signal.reason), { once: true }))
    throw new Error('unreachable')
  } })
  host.handleMessage({ type: 'artifact_resolve', id: 'request', jobId: start.jobId, manifestHash: catalog.hash, target: 'aarch64-apple-darwin', sha256: descriptor.targets['aarch64-apple-darwin'].sha256 })
  expect(observed?.aborted).toBe(false)
  host.handleMessage({ type: 'job_exit', jobId: start.jobId, exitCode: 0 })
  await Bun.sleep(0)
  expect(observed?.aborted).toBe(true)
  expect(messages.filter(message => message.type === 'artifact_location')).toHaveLength(0)
  host.detach()
})
