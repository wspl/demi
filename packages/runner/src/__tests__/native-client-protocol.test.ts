import { test, expect } from 'bun:test'
import { createServer, type Socket } from 'node:net'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { commandClientBinary } from '../../../command-client/build'
import { LOCAL, localFrame, localFrames, localInvokeSchema, localWatchSchema } from '@demicodes/runner-protocol/local'

async function peer(run: (data: Socket, control: Socket) => void | Promise<void>) {
  const path = await mkdtemp(join(tmpdir(), 'demi-client-wire-'))
  const endpoint = join(path, 'ipc.sock')
  const sockets = new Set<Socket>()
  const handlers: Promise<void>[] = []
  const failures: unknown[] = []
  let data: Socket | undefined
  const server = createServer(socket => {
    sockets.add(socket)
    socket.on('close', () => sockets.delete(socket))
    socket.on('error', () => {})
    const handled = (async () => {
      for await (const frame of localFrames(socket)) {
        if (frame.type === LOCAL.frames.invoke) {
          const request = localInvokeSchema.parse(JSON.parse(new TextDecoder().decode(frame.body)))
          expect(request.argv).toEqual(['probe', '你好'])
          data = socket
          socket.write(localFrame(LOCAL.frames.ready))
        } else {
          expect(frame.type).toBe(LOCAL.frames.watch)
          localWatchSchema.parse(JSON.parse(new TextDecoder().decode(frame.body)))
          socket.write(localFrame(LOCAL.frames.ready))
          if (!data) throw new Error('control connected before invocation')
          await run(data, socket)
        }
      }
    })().catch(error => {
      // These cases intentionally close sockets with unread output. Only that reset is expected.
      if (!(error instanceof Error && 'code' in error && error.code === 'ECONNRESET')) {
        failures.push(error)
        socket.destroy()
      }
    })
    handlers.push(handled)
  })
  await new Promise<void>(resolve => server.listen(endpoint, resolve))
  const child = Bun.spawn([commandClientBinary(), 'probe', '你好'], {
    env: { ...process.env, DEMI_RUNNER_ENDPOINT: endpoint, DEMI_CONTEXT_ID: 'a'.repeat(32) },
    stdin: 'pipe',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  return {
    child,
    async close() {
      child.kill()
      await child.exited
      for (const socket of sockets) socket.destroy()
      await new Promise<void>(resolve => server.close(() => resolve()))
      await Promise.all(handlers)
      await rm(path, { recursive: true, force: true })
      if (failures.length) throw new AggregateError(failures, 'native client peer failed')
    },
  }
}

test('native decoder accepts fragmented binary frames and preserves final exit status', async () => {
  const fixture = await peer(async data => {
    const bytes = localFrame(LOCAL.frames.stdout, new Uint8Array([0, 255, 10, 128]))
    for (const byte of bytes) {
      data.write(new Uint8Array([byte]))
      await Bun.sleep(1)
    }
    data.write(localFrame(LOCAL.frames.stderr, new TextEncoder().encode('diagnostic')))
    const exit = new Uint8Array(4)
    new DataView(exit.buffer).setUint32(0, 23)
    data.write(localFrame(LOCAL.frames.exit, exit))
  })
  try {
    const [stdout, stderr, code] = await Promise.all([new Response(fixture.child.stdout).bytes(), new Response(fixture.child.stderr).text(), fixture.child.exited])
    expect(stdout).toEqual(new Uint8Array([0, 255, 10, 128]))
    expect(stderr).toBe('diagnostic')
    expect(code).toBe(23)
  } finally {
    await fixture.close()
  }
})
test('native decoder rejects an oversized frame before buffering its body', async () => {
  const fixture = await peer(data => {
    const frame = new Uint8Array(5)
    new DataView(frame.buffer).setUint32(0, LOCAL.maxFrame + 1)
    frame[4] = LOCAL.frames.stdout
    data.write(frame)
  })
  try {
    expect(await fixture.child.exited).toBe(1)
    expect(await new Response(fixture.child.stderr).text()).toContain('frame exceeds limit')
  } finally {
    await fixture.close()
  }
})
test('native client fails if runner disconnects without an exit frame', async () => {
  const fixture = await peer(data => { data.end() })
  try {
    expect(await fixture.child.exited).toBe(1)
    expect(await new Response(fixture.child.stderr).text()).toContain('before exit')
  } finally {
    await fixture.close()
  }
})
test('control disconnect cancels a native client while stdout is backpressured', async () => {
  const fixture = await peer((data, control) => {
    const output = localFrame(LOCAL.frames.stdout, new Uint8Array(LOCAL.chunkSize))
    for (let i = 0; i < 100; i++) data.write(output)
    setTimeout(() => control.end(), 50)
  })
  try {
    expect(await fixture.child.exited).toBe(1)
    expect(await new Response(fixture.child.stderr).text()).toContain('before exit')
  } finally {
    await fixture.close()
  }
})
