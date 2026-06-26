import { delay } from '@demi/utils'
import process from 'node:process'

const KILL_WAIT_MS = 100
const KILL_RETRIES = 20

export async function killListeningPorts(ports: readonly number[]): Promise<void> {
  for (const port of ports) await killListeningPort(port)
}

export async function killListeningPort(port: number): Promise<void> {
  if (port === 0) return

  const pids = await listeningPids(port)
  const targets = pids.filter((pid) => pid !== process.pid)
  if (targets.length === 0) return

  await run(['kill', ...targets.map(String)])
  if (await waitUntilFree(port)) return

  const stubborn = (await listeningPids(port)).filter((pid) => pid !== process.pid)
  if (stubborn.length > 0) await run(['kill', '-9', ...stubborn.map(String)])
  if (await waitUntilFree(port)) return

  throw new Error(`Port ${port} is still in use after killing existing listeners`)
}

async function waitUntilFree(port: number): Promise<boolean> {
  for (let attempt = 0; attempt < KILL_RETRIES; attempt++) {
    if ((await listeningPids(port)).length === 0) return true
    await delay(KILL_WAIT_MS)
  }
  return false
}

async function listeningPids(port: number): Promise<number[]> {
  const proc = Bun.spawn(['lsof', `-tiTCP:${port}`, '-sTCP:LISTEN'], {
    stdout: 'pipe',
    stderr: 'ignore',
  })
  const stdout = await new Response(proc.stdout).text()
  await proc.exited
  return stdout
    .split(/\s+/)
    .map((value) => Number(value))
    .filter((value) => Number.isInteger(value) && value > 0)
}

async function run(cmd: string[]): Promise<void> {
  const proc = Bun.spawn(cmd, { stdout: 'ignore', stderr: 'ignore' })
  const exitCode = await proc.exited
  if (exitCode !== 0) throw new Error(`${cmd.join(' ')} failed with exit code ${exitCode}`)
}
