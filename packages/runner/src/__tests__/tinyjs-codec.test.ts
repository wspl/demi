import { expect, test } from 'bun:test'
import { mkdtemp, realpath } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { bundleForTinyjs, tinyjsBinary } from '../testing'
import { RUNNER_PROTOCOL_VERSION, createRunnerWire, type RunnerProtocolMessage } from '@demicodes/runner-protocol'
import { msgpackCodec } from '@demicodes/runner-protocol/msgpack'

// The two codecs of the wire, @msgpack/msgpack on Bun and tinyjs:bytes on
// tinyjs, must produce and read the same bytes: frames encoded here are
// decoded, re-encoded and returned by tinyjs, then compared byte for byte.
test('frames encoded on Bun are read and reproduced identically by tinyjs', async () => {
  const wire = createRunnerWire(msgpackCodec)
  const messages: RunnerProtocolMessage[] = [
    { type: 'hello', protocol: RUNNER_PROTOCOL_VERSION, deviceToken: 't', runner: { name: 'n', platform: 'p', version: '1', identity: { uid: 501, gid: 20, hostname: 'h', homeDir: '/h' } } },
    { type: 'rpc_cancel', callId: 'cancelled-call' },
    { type: 'pong', jobs: 3 },
    { type: 'fs_utimes', id: 'a', path: '/x', atime: new Date(1_600_000_000_000), mtime: new Date(1_600_000_000_250), cwd: '/' },
    { type: 'fs_ok', id: 'a', op: 'readFile', result: new Uint8Array([0, 1, 254, 255]) },
    { type: 'fs_ok', id: 'b', op: 'stat', result: { isFile: true, isDirectory: false, isSymbolicLink: false, mode: 0o100644, size: 2 ** 40, mtime: new Date(-1000) } },
    { type: 'fs_error', id: 'c', code: 'ENOENT', message: 'no' },
    { type: 'spawn_output', spawnId: 's', stream: 'stderr', bytes: new Uint8Array(70_000).fill(7) },
    { type: 'spawn_exit', spawnId: 's', exitCode: null, signal: 'SIGKILL', spawnError: { kind: 'other' } },
  ]
  const work = await realpath(await mkdtemp(join(tmpdir(), 'demi-codec-')))
  await Bun.write(join(work, 'entry.ts'), `
import { msgpackEncode, msgpackDecode } from 'tinyjs:bytes'
import * as fs from 'tinyjs:fs'
import { env, exit } from 'tinyjs:runtime'
const frames = msgpackDecode(await fs.readFile(env.FRAMES)) as Uint8Array[]
await fs.writeFile(env.FRAMES_BACK, msgpackEncode(frames.map((frame) => msgpackEncode(msgpackDecode(frame)))))
exit(0)
`)
  const bundle = join(work, 'entry.mjs')
  await bundleForTinyjs(join(work, 'entry.ts'), bundle)
  const frames = messages.map((message) => wire.encode(message))
  await Bun.write(join(work, 'frames.bin'), msgpackCodec.encode(frames))
  const run = Bun.spawnSync([tinyjsBinary(), bundle], {
    env: { PATH: process.env.PATH ?? '', FRAMES: join(work, 'frames.bin'), FRAMES_BACK: join(work, 'back.bin') },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  expect(run.exitCode, run.stderr.toString()).toBe(0)
  const back = msgpackCodec.decode(new Uint8Array(await Bun.file(join(work, 'back.bin')).arrayBuffer())) as Uint8Array[]
  expect(back.length).toBe(frames.length)
  for (const [index, frame] of frames.entries()) {
    expect(Buffer.from(back[index]!).equals(Buffer.from(frame)), `frame ${index} (${messages[index]!.type})`).toBe(true)
    const runnerToBackend = ['hello', 'pong', 'fs_ok', 'fs_error', 'spawn_output', 'spawn_exit', 'rpc_cancel'].includes(messages[index]!.type)
    const decoded = runnerToBackend ? wire.decodeRunnerToBackend(back[index]!) : wire.decodeBackendToRunner(back[index]!)
    expect(decoded).toEqual(messages[index])
  }
}, 60_000)
