import { test, expect } from 'bun:test'
import { mkdtemp, mkdir, copyFile, writeFile, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, dirname } from 'node:path'
import { createHash } from 'node:crypto'
import { packedRunner } from '@demicodes/runner/testing'
import { runnerInstallRoutes, type RunnerRelease } from '../http/runner-install'
import { RUNNER_PROTOCOL_VERSION } from '@demicodes/runner-protocol'
import { LOCAL } from '@demicodes/runner-protocol/local'

const sha = (data: Uint8Array | string) => createHash('sha256').update(data).digest('hex')
async function run(args: string[], home: string, extra: Record<string, string> = {}) {
  const child = Bun.spawn(args, { env: { ...process.env, HOME: home, ...extra }, stdout: 'pipe', stderr: 'pipe' })
  const [out, err, code] = await Promise.all([new Response(child.stdout).text(), new Response(child.stderr).text(), child.exited])
  return { code, out, err }
}
test('installer keeps backend registrations separate, reuses a release, and drains only its own upgrade', async () => {
  const work = await mkdtemp(join(tmpdir(), 'demi-install-test-'))
  const home = join(work, 'home'), artifacts = join(work, 'artifacts')
  await mkdir(home)
  const runner = await packedRunner(), client = join(dirname(runner), 'demi')
  const platform = `${process.platform === 'darwin' ? 'macos' : 'linux'}-${process.arch === 'arm64' ? 'arm64' : 'x64'}` as keyof RunnerRelease['targets']
  const hashes = { runner: sha(await readFile(runner)), client: sha(await readFile(client)) }
  async function publish(name: string) {
    const release = sha(name), target = join(artifacts, release, platform)
    await mkdir(target, { recursive: true })
    await copyFile(runner, join(target, 'demi-runner')); await copyFile(client, join(target, 'demi'))
    const manifest = JSON.stringify({ release, wire: RUNNER_PROTOCOL_VERSION, local: LOCAL.version, targets: { [platform]: hashes } })
    await writeFile(join(artifacts, release, 'manifest.json'), manifest)
    await writeFile(join(artifacts, 'manifest.json'), manifest)
    return release
  }
  const initial = await publish('initial')
  const app = runnerInstallRoutes({ directory: artifacts })
  const a = Bun.serve({ port: 0, fetch: app.fetch }), b = Bun.serve({ port: 0, fetch: app.fetch })
  const state = (port: number) => join(home, '.demi/instances', sha(`http://localhost:${port}/`))
  const active = async (port: number) => JSON.parse(await readFile(join(state(port), 'active.json'), 'utf8'))
  async function install(port: number, extra: Record<string, string> = {}) {
    const script = join(work, `install-${port}.sh`)
    await writeFile(script, await (await fetch(`http://localhost:${port}/install.sh`)).text())
    const result = await run(['sh', script], home, extra)
    if (result.code) throw new Error(JSON.stringify(result))
    return result
  }
  try {
    await install(a.port!); await install(b.port!)
    const firstA = await active(a.port!), firstB = await active(b.port!)
    expect(firstA.endpoint).not.toBe(firstB.endpoint)
    expect(firstA.release).toBe(initial)
    expect((await install(a.port!)).out).toContain('already running')
    expect((await active(a.port!)).endpoint).toBe(firstA.endpoint)
    const collision = await run(['sh', join(work, `install-${b.port}.sh`)], home, { DEMI_INSTALLATION_ID: sha(`http://localhost:${a.port}/`) })
    expect(collision.code).toBe(1)
    expect(collision.err).toContain('another backend')
    const upgraded = await publish('upgraded')
    await install(a.port!)
    expect((await fetch(`http://localhost:${a.port}/runner-artifacts/${initial}/${platform}/demi`)).status).toBe(200)
    expect((await active(a.port!)).release).toBe(upgraded)
    expect((await active(a.port!)).endpoint).not.toBe(firstA.endpoint)
    expect((await active(b.port!)).endpoint).toBe(firstB.endpoint)
  } finally {
    for (const port of [a.port!, b.port!]) await run([join(state(port), 'run'), 'drain'], home).catch(() => {})
    a.stop(true); b.stop(true)
    await rm(work, { recursive: true, force: true })
  }
}, 60_000)
