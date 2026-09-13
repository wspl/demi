import { test, expect } from 'bun:test'
import {
  mkdtemp,
  mkdir,
  copyFile,
  writeFile,
  readFile,
  rm
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { runnerBinary } from '@demicodes/runner/testing'
import { runnerInstallRoutes, type RunnerRelease } from '../http/runner-install'
import { RUNNER_PROTOCOL_VERSION } from '@demicodes/runner-protocol'
import { NATIVE_PROTOCOL_VERSION, NATIVE_TARGETS } from '@demicodes/command-protocol'

const sha = (data: Uint8Array | string) => createHash('sha256')
  .update(data)
  .digest('hex')
async function run(
  args: string[],
  home: string,
  extra: Record<string, string> = {}
) {
  const logs = await mkdtemp(join(home, '.installer-output-'))
  const stdout = join(logs, 'stdout')
  const stderr = join(logs, 'stderr')
  // Detached Windows children may inherit pipe handles. The installer exit,
  // not EOF on a descendant's inherited handle, defines invocation completion.
  // The fixture removes these logs after draining all detached runners.
  const child = Bun.spawn(args, {
    env: { ...process.env, HOME: home, USERPROFILE: home, ...extra },
    stdout: Bun.file(stdout),
    stderr: Bun.file(stderr),
    timeout: 60_000,
  })
  const code = await child.exited
  const [out, err] = await Promise.all([
    readFile(stdout, 'utf8'),
    readFile(stderr, 'utf8'),
  ])
  return { code, out, err }
}
test(
  'installer keeps backend registrations separate, reuses a release, and drains only its own upgrade',
  async () => {
    const work = await mkdtemp(join(tmpdir(), 'demi-install-test-'))
    const home = join(work, 'home')
    const artifacts = join(work, 'artifacts')
    const windows = process.platform === 'win32'
    const executable = windows ? 'demi-runner.exe' : 'demi-runner'
    const extension = windows ? 'ps1' : 'sh'
    const launchScript = (script: string, args: string[] = []) => windows
      ? ['powershell', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, ...args]
      : ['sh', script, ...args]
    await mkdir(home)
    const runner = await runnerBinary()
    const platform = `${process.arch === 'arm64' ? 'aarch64' : 'x86_64'}-${windows ? 'pc-windows-msvc' : process.platform === 'darwin' ? 'apple-darwin' : 'unknown-linux-musl'}` as keyof RunnerRelease['targets']
    const bytes = await readFile(runner)
    const artifact = { sha256: sha(bytes), size: bytes.length }
    async function publish(name: string) {
      const release = sha(name)
      const target = join(artifacts, release, platform)
      await mkdir(target, { recursive: true })
      await copyFile(runner, join(target, executable))
      const manifest = JSON.stringify({
        release,
        wire: RUNNER_PROTOCOL_VERSION,
        commandProtocol: NATIVE_PROTOCOL_VERSION,
        // Test-only catalog: only the current host executable is served.
        targets: Object.fromEntries(NATIVE_TARGETS.map(target => [target, artifact]))
      })
      await writeFile(join(artifacts, release, 'manifest.json'), manifest)
      await writeFile(join(artifacts, 'manifest.json'), manifest)
      return release
    }
    const initial = await publish('initial')
    const app = runnerInstallRoutes({ directory: artifacts })
    const a = Bun.serve({ port: 0, fetch: app.fetch })
    const b = Bun.serve({ port: 0, fetch: app.fetch })
    const state = (port: number) => join(
      home,
      '.demi/instances',
      sha(`http://localhost:${port}/`)
    )
    const active = async (port: number) => JSON.parse(
      await readFile(join(state(port), 'active.json'), 'utf8')
    )
    async function install(port: number, extra: Record<string, string> = {}) {
      const script = join(work, `install-${port}.${extension}`)
      await writeFile(
        script,
        await (await fetch(`http://localhost:${port}/install.${extension}`)).text()
      )
      const result = await run(launchScript(script), home, extra)
      if (result.code)
        throw new Error(JSON.stringify(result))
      return result
    }
    try {
      const windowsInstaller = await fetch(`http://localhost:${a.port}/install.ps1`)
      expect(windowsInstaller.status).toBe(200)
      expect(await windowsInstaller.text()).toContain('aarch64-pc-windows-msvc')
      await install(a.port!)
      await install(b.port!)
      const firstA = await active(a.port!)
      const firstB = await active(b.port!)
      expect(firstA.endpoint).not.toBe(firstB.endpoint)
      expect(firstA.release).toBe(initial)
      expect((await install(a.port!)).out).toContain('already running')
      expect((await active(a.port!)).endpoint).toBe(firstA.endpoint)
      const collision = await run(launchScript(
        join(work, `install-${b.port}.${extension}`)
      ), home, { DEMI_INSTALLATION_ID: sha(`http://localhost:${a.port}/`) })
      expect(collision.code).toBe(1)
      expect(collision.err).toContain('another backend')
      const upgraded = await publish('upgraded')
      await install(a.port!)
      expect(
        (await fetch(
          `http://localhost:${a.port}/runner-artifacts/${initial}/${platform}/${executable}`
        )).status
      )
        .toBe(200)
      expect((await active(a.port!)).release).toBe(upgraded)
      expect((await active(a.port!)).endpoint).not.toBe(firstA.endpoint)
      expect((await active(b.port!)).endpoint).toBe(firstB.endpoint)
    } finally {
      for (const port of [a.port!, b.port!]) {
        // A failed installation may not have produced its launcher; teardown
        // still stops the HTTP fixtures and removes their temporary directory.
        const command = windows
          ? launchScript(join(state(port), 'run.ps1'), ['-Action', 'drain'])
          : [join(state(port), 'run'), 'drain']
        await run(command, home).catch(() => {})
      }
      a.stop(true)
      b.stop(true)
      await rm(work, { recursive: true, force: true })
    }
  },
  180_000
)
