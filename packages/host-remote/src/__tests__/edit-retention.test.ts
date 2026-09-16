import { expect, test } from 'bun:test'
import { memoryHostStore } from '@demicodes/shell/testing'
import { delay } from '@demicodes/utils'
import { RemoteHost, RemoteShellEnvironment } from '../index'

test('completion waits for publication and later polls retain UI metadata', async () => {
  const host = new RemoteHost({ defaultCwd: '/work', identity: { uid: 1, gid: 1, hostname: 'test', homeDir: '/work' }, store: memoryHostStore() })
  let jobId = ''
  host.attach(message => {
    if (message.type === 'job_start') {
      jobId = message.jobId
    }
  })
  let publish!: () => void
  const barrier = new Promise<void>(resolve => { publish = resolve })
  const file = { path: '/work/file', kind: 'modified' as const, added: 1, removed: 1, edits: [{ kept: true }] }
  const shell = new RemoteShellEnvironment({ conversation: 'test-conversation', node: 'test-session', host, retainEdits: async () => { await barrier; return [file] } })
  try {
    const started = await shell.exec({ script: 'echo new > file', timeoutMs: 1 })
    host.handleMessage({ type: 'job_exit', jobId, exitCode: 7, filesTruncated: true,
      files: [{ ...file, edits: [{ original: '/copies/before', modified: '/copies/after' }] }] })
    await delay(1)
    expect((await shell.status({ commandId: started.commandId })).status).toBe('running')
    publish()
    for (let attempt = 0; attempt < 100; attempt += 1) {
      if ((await shell.status({ commandId: started.commandId })).status !== 'running') {
        break
      }
      await delay(10)
    }
    const result = await shell.status({ commandId: started.commandId })
    expect(result).toMatchObject({ status: 'exited', exitCode: 7, files: [file], filesTruncated: true })
    expect((await shell.status({ commandId: started.commandId })).files).toEqual([file])
  } finally {
    publish()
    await shell.disposeAllShells()
  }
})
