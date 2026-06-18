import { expect, test } from 'bun:test'
import { LocalHost } from '../local-host'

test('LocalHost spawns a command and captures stdout', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.spawn({ command: 'printf', args: ['hello\\n'] })

  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])

  expect(stdout).toBe('hello\n')
  expect(exit).toEqual({ exitCode: 0, signal: undefined })
})

test('LocalHost writes stdin to a spawned process', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.spawn({
    command: 'sh',
    args: ['-c', 'IFS= read -r line; printf "%s" "$line"'],
  })

  await handle.writeStdin(Buffer.from('from stdin\n'))
  await handle.closeStdin()
  const [stdout, exit] = await Promise.all([collectText(handle.stdout), handle.wait()])

  expect(stdout).toBe('from stdin')
  expect(exit.exitCode).toBe(0)
})

test('LocalHost can terminate a foreground process', async () => {
  const host = new LocalHost(process.cwd())
  const handle = await host.spawn({ command: 'sleep', args: ['10'] })

  await handle.kill()
  const exit = await handle.wait()

  expect(exit.exitCode).toBeNull()
  expect(exit.signal).toBe('SIGTERM')
})

async function collectText(iterable: AsyncIterable<Uint8Array>): Promise<string> {
  const chunks: Uint8Array[] = []
  for await (const chunk of iterable) chunks.push(chunk)
  return Buffer.concat(chunks).toString('utf8')
}
