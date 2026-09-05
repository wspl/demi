import { expect, test } from 'bun:test'
import { createRunnerWire, type BackendToRunnerMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'
import { memoryHostStore } from '@demicodes/shell/testing'
import { RemoteHost, RemoteShellEnvironment } from '../index'

test('remote statuses track active invocation hints independently and discard them at job exit', async () => {
  const host = new RemoteHost({ defaultCwd: '/work', identity: { uid: 1, gid: 1, hostname: 'test', homeDir: '/work' }, store: memoryHostStore() })
  const sent: BackendToRunnerMessage[] = []
  host.attach((message) => {
    sent.push(message)
    if (message.type === 'job_kill') host.handleMessage({ type: 'job_exit', jobId: message.jobId, exitCode: null, signal: 'SIGTERM' })
  })
  const shell = new RemoteShellEnvironment({ host })
  const wire = createRunnerWire(msgpackCodec)
  try {
    const started = await shell.exec({ script: 'attend first | attend second', timeoutMs: 1 })
    const job = sent.find((message) => message.type === 'job_start')!
    if (job.type !== 'job_start') throw new Error('job not started')
    const hint = (invocationId: string, value: string | null, jobId = job.jobId) => host.handleMessage(wire.decodeRunnerToBackend(wire.encode({ type: 'job_running_hint', jobId, invocationId, hint: value })))
    hint('foreign', 'another job', 'not-this-job')
    expect('runningHint' in await shell.status({ commandId: started.commandId })).toBe(false)
    hint('first', 'first hint')
    hint('second', 'second hint')
    let view = await shell.status({ commandId: started.commandId })
    expect(view.status === 'running' && view.runningHint).toBe('second hint')
    hint('second', null)
    view = await shell.status({ commandId: started.commandId })
    expect(view.status === 'running' && view.runningHint).toBe('first hint')
    hint('first', null)
    expect('runningHint' in await shell.status({ commandId: started.commandId })).toBe(false)
    hint('third', 'hint before abort')
    const aborted = await shell.abort({ commandId: started.commandId })
    expect(aborted.status).toBe('aborted')
    expect('runningHint' in aborted).toBe(false)
    hint('late', 'arrived after exit')
    expect('runningHint' in await shell.status({ commandId: started.commandId })).toBe(false)
  } finally { await shell.disposeAllShells() }
})

test('disconnect clears a remote job hint with the failed job', async () => {
  const host = new RemoteHost({ defaultCwd: '/work', identity: { uid: 1, gid: 1, hostname: 'test', homeDir: '/work' }, store: memoryHostStore() })
  let jobId = ''
  host.attach((message) => { if (message.type === 'job_start') jobId = message.jobId })
  const job = host.startJob({ script: 'attend', cwd: '/work', env: {} })
  host.handleMessage({ type: 'job_running_hint', jobId, invocationId: 'i1', hint: 'attending' })
  expect(job.runningHint).toBe('attending')
  host.detach()
  expect((await job.wait()).spawnError?.kind).toBe('other')
  expect(job.runningHint).toBeUndefined()
})
