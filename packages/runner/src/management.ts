import { LOCAL, localFrame, localFrames } from '@demicodes/runner-protocol/local'
import { activeRunnerSchema } from './state'
import { connectUnix, createRunnerHost, env, stdoutWriter } from './machine'
import { decodeUtf8, encodeUtf8, delay } from '@demicodes/utils'

/** Installation-local administration; never selects a different backend on failure. */
export async function manageRunner(dir: string, action: 'status' | 'drain'): Promise<number> {
  const host = createRunnerHost()
  const active = activeRunnerSchema.parse(JSON.parse(decodeUtf8(await host.fs.readFile(`${dir}/active.json`))))
  const socket = await connectUnix(active.endpoint)
  try {
    await socket.write(localFrame(LOCAL.frames.manage, encodeUtf8(JSON.stringify({ version: LOCAL.version, secret: active.secret, action }))))
    let ready = false
    for await (const reply of localFrames(socket.input)) {
      if (reply.type === LOCAL.frames.error) throw new Error(decodeUtf8(reply.body))
      if (reply.type !== LOCAL.frames.ready || reply.body.length) throw new Error('invalid management response')
      ready = true
      break
    }
    if (!ready) throw new Error('runner disconnected before management response')
    if (action === 'status') {
      await stdoutWriter()(`running ${active.release}\n`)
      return env.DEMI_RELEASE_ID && active.release !== env.DEMI_RELEASE_ID ? 3 : 0
    }
  } finally { socket.close() }
  // Wait for the old daemon to release its OS lock before installing a replacement.
  const file = await tjs.open(`${dir}/runner.lock`, 'a', 0o600)
  try { while (!file.lock()) await delay(50) } finally { await file.close() }
  return 0
}
