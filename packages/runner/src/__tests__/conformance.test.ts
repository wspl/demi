import { expect, test } from 'bun:test'
import { mkdtemp, realpath, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { RemoteHost } from '@demicodes/host-remote'
import { hostConformanceCases, memoryHostStore } from '@demicodes/shell/testing'
import { connectTestRunner } from '../testing'

test('the Rust runner passes the Host conformance suite over its actual wire', async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'demi-conformance-')))
  const host = new RemoteHost({
    defaultCwd: root,
    identity: { uid: 0, gid: 0, hostname: 'test', homeDir: root },
    store: memoryHostStore(),
  })
  const connection = await connectTestRunner({
    home: root,
    onHello: (send, hello) => host.attach(send, hello.runner.identity),
    onMessage: message => host.handleMessage(message),
    onClose: () => host.detach(),
  })
  try {
    for (const check of hostConformanceCases({ host, root, path: process.env.PATH })) {
      try {
        await check.run()
      } catch (error) {
        throw new Error(check.name, { cause: error })
      }
    }
    expect(host.online).toBe(true)
  } finally {
    await connection.close()
    await rm(root, { recursive: true, force: true })
  }
}, 120_000)
