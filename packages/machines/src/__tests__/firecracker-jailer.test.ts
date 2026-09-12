import { expect, test } from 'bun:test'
import { existsSync, readFileSync, statSync } from 'node:fs'
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile
} from 'node:fs/promises'
import { createConnection } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { waitFor, withTimeout } from '@demicodes/utils'

// These tests exercise real Linux ownership, Unix sockets and process lifetimes, without KVM.
const linuxRoot = process.platform === 'linux' &&
  process.getuid?.() === 0
  ? test
  : test.skip
const script = resolve(import.meta.dir, '../../scripts/firecracker-jailer.sh')
const fakeJailer = resolve(import.meta.dir, 'fixtures/fake-jailer.ts')
const uid = 10001
const gid = 10002
const backendGid = 10003

type Mode = 'run' | 'fail' | 'no-pid' | 'no-socket' | 'hold-parent'

async function processRunning(pid: number): Promise<boolean> {
  try {
    const text = await readFile(`/proc/${pid}/stat`, 'utf8')
    const state = text.slice(text.lastIndexOf(')') + 2).split(' ')[0]
    return state !== 'Z' && state !== 'X'
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT')
      return false
    throw error
  }
}

function launch(commandArgs: string[]) {
  const child = Bun.spawn(['/bin/bash', script, ...commandArgs], {
    stdin: 'ignore',
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const stderr = new Response(child.stderr).text()
  const stdout = new Response(child.stdout).text()
  return { child, stderr, stdout }
}


async function fixture(mode: Mode = 'run') {
  const directory = await mkdtemp(join(tmpdir(), 'demi-jailer-tests-'))
  const executableDir = join(directory, 'test tools')
  await mkdir(executableDir)
  const jailer = join(executableDir, 'fake jailer')
  const firecracker = join(executableDir, 'firecracker-test')
  await writeFile(
    jailer,
    `#!${process.execPath}\nimport ${JSON.stringify(pathToFileURL(fakeJailer).href)}\n`
  )
  await chmod(jailer, 0o755)
  await writeFile(firecracker, 'fake executable')
  await chmod(firecracker, 0o755)
  await writeFile(
    join(executableDir, 'scenario.json'),
    JSON.stringify({ directory, mode })
  )

  const images = join(directory, 'working images')
  await mkdir(images)
  const kernel = join(images, 'vmlinux')
  const rootfs = join(images, 'rootfs.ext4')
  const home = join(images, 'home.ext4')
  const system = join(images, 'system.ext4')
  for (const [path, contents] of [
    [kernel, 'kernel'],
    [rootfs, 'rootfs'],
    [home, 'home'],
    [system, 'system']
  ] as const) {
    await writeFile(path, contents)
  }
  const chrootBase = join(directory, 'jails')
  const id = 'vm-test'
  const jail = join(chrootBase, 'firecracker-test', id)
  const root = join(jail, 'root')
  const socket = join(root, 'run/firecracker.socket')
  const args = [
    'vm', 'start',
    '--id', id,
    '--jailer', jailer,
    '--firecracker', firecracker,
    '--chroot-base', chrootBase,
    '--uid', String(uid),
    '--gid', String(gid),
    '--backend-gid', String(backendGid),
    '--kernel', kernel,
    '--rootfs', rootfs,
    '--home', home,
    '--system', system,
  ]
  const children: ReturnType<typeof launch>[] = []


  function start(commandArgs?: string[]) {
    const child = launch(commandArgs ?? args)
    children.push(child)
    return child
  }

  async function pid(kind: 'launcher' | 'vm'): Promise<number> {
    return JSON.parse(await readFile(join(directory, `${kind}.json`), 'utf8')).pid
  }

  return {
    directory,
    id,
    args,
    jail,
    root,
    socket,
    chrootBase,
    images,
    kernel,
    rootfs,
    home,
    system,
    start,
    pid,
    async waitForVm(): Promise<number> {
      await waitFor(
        () => existsSync(join(directory, 'vm.json')) && existsSync(socket),
        undefined,
        { timeoutMs: 5_000 }
      )
      const vmPid = await pid('vm')
      return vmPid
    },
    async exitVm(): Promise<void> {
      await new Promise<void>((resolve, reject) => {
        const connection = createConnection(
          socket,
          () => connection.end('exit')
        )
        connection.once('error', reject)
        connection.once('close', () => resolve())
      })
    },
    async close(): Promise<void> {
      try {
        for (const { child } of children) {
          if (child.exitCode === null)
            child.kill('SIGTERM')
        }
        for (const kind of ['launcher', 'vm'] as const) {
          if (!existsSync(join(directory, `${kind}.json`)))
            continue
          const target = await pid(kind)
          if (await processRunning(target))
            process.kill(target, 'SIGKILL')
        }
        await Promise.all(children.map(async ({ child, stderr, stdout }) => {
          await withTimeout(
            child.exited,
            5_000,
            'helper did not stop during test cleanup'
          )
          await Promise.all([stderr, stdout])
        }))
      } finally {
        for (const { child } of children) {
          if (child.exitCode === null)
            child.kill('SIGKILL')
        }
        await rm(directory, { recursive: true, force: true })
      }
    },
  }
}

async function expectExit(
  process: ReturnType<typeof launch>,
  expected: number
): Promise<void> {
  const code = await withTimeout(
    process.child.exited,
    6_000,
    'jailer helper did not exit'
  )
  if (code !== expected) {
    const diagnostic = await withTimeout(process.stderr, 1_000, 'helper stderr remained open')
      .catch(String)
    throw new Error(
      `helper exited ${code}, expected ${expected}:\n${diagnostic}`
    )
  }
  expect(code).toBe(expected)
}

async function waitForPublishedVm(
  f: Awaited<ReturnType<typeof fixture>>,
  vmPid: number
): Promise<void> {
  await waitFor(() => {
    try {
      if (Number(readFileSync(join(f.jail, 'pid'), 'utf8')) !== vmPid)
        return false
      return (statSync(f.socket).mode & 0o777) === 0o660
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === 'ENOENT')
        return false
      throw error
    }
  }, undefined, { timeoutMs: 5_000 })
}

linuxRoot(
  'jailer start stages shared disks, exposes the API socket, and follows the VM after its launcher exits',
  async () => {
    const f = await fixture()
    try {
      const started = f.start()
      const vmPid = await f.waitForVm()
      await waitForPublishedVm(f, vmPid)
      expect(await processRunning(await f.pid('launcher'))).toBe(false)
      expect(await processRunning(vmPid)).toBe(true)
      expect(started.child.exitCode).toBeNull()
      expect(Number(await readFile(join(f.jail, 'pid'), 'utf8'))).toBe(vmPid)
      const launched = JSON.parse(
        await readFile(join(f.directory, 'launcher.json'), 'utf8')
      )
      expect(launched.args).toContain('--new-pid-ns')
      expect(launched.args.slice(-3)).toEqual([
        '--cgroup-version',
        '2',
        '--new-pid-ns'
      ])
      expect(await readFile(join(f.root, 'vmlinux'), 'utf8')).toBe('kernel')
      expect(await readFile(join(f.root, 'rootfs.ext4'), 'utf8')).toBe('rootfs')
      for (const image of ['home', 'system']) {
        const source = await stat(join(f.images, `${image}.ext4`))
        const staged = await stat(join(f.root, `${image}.ext4`))
        expect(staged.ino).toBe(source.ino)
        expect(staged.dev).toBe(source.dev)
        expect(staged.uid).toBe(uid)
        expect(staged.gid).toBe(backendGid)
        expect(staged.mode & 0o777).toBe(0o660)
      }
      for (const path of [f.root, join(f.root, 'run')]) {
        const directory = await stat(path)
        expect(directory.gid).toBe(backendGid)
        expect(directory.mode & 0o777).toBe(0o750)
      }
      const apiSocket = await stat(f.socket)
      expect(apiSocket.isSocket()).toBe(true)
      expect(apiSocket.gid).toBe(backendGid)
      expect(apiSocket.mode & 0o777).toBe(0o660)
      await f.exitVm()
      await expectExit(started, 0)
      expect(existsSync(f.jail)).toBe(false)
      expect(await readFile(f.home, 'utf8')).toBe('home')
      expect(await readFile(f.system, 'utf8')).toBe('system')
    } finally {
      await f.close()
    }
  },
  15_000
)

linuxRoot(
  'jailer kill stops the namespace process and releases its waiting start command',
  async () => {
    const f = await fixture()
    try {
      const started = f.start()
      const vmPid = await f.waitForVm()
      await waitForPublishedVm(f, vmPid)
      const killed = f.start([
        'vm',
        'kill',
        '--id',
        f.id,
        '--chroot-base',
        f.chrootBase
      ])
      await expectExit(killed, 0)
      await expectExit(started, 0)
      expect(await processRunning(vmPid)).toBe(false)
      expect(existsSync(f.jail)).toBe(false)
      const again = f.start([
        'vm',
        'kill',
        '--id',
        f.id,
        '--chroot-base',
        f.chrootBase
      ])
      await expectExit(again, 1)
    } finally {
      await f.close()
    }
  },
  15_000
)

for (const mode of ['run', 'hold-parent'] as const) {
  linuxRoot(
    `cancelling start kills both jailer and VM (${mode}) without deleting working images`,
    async () => {
      const f = await fixture(mode)
      try {
        const started = f.start()
        const vmPid = await f.waitForVm()
        const launcherPid = await f.pid('launcher')
        if (mode === 'run')
          await waitForPublishedVm(f, vmPid)
        started.child.kill('SIGTERM')
        await expectExit(started, 143)
        expect(await processRunning(launcherPid)).toBe(false)
        expect(await processRunning(vmPid)).toBe(false)
        expect(existsSync(f.jail)).toBe(false)
        expect(await readFile(f.home, 'utf8')).toBe('home')
        expect(await readFile(f.system, 'utf8')).toBe('system')
      } finally {
        await f.close()
      }
    },
    15_000
  )
}

linuxRoot(
  'a duplicate start cannot replace or remove an existing jail',
  async () => {
    const f = await fixture()
    try {
      const first = f.start()
      const vmPid = await f.waitForVm()
      await waitForPublishedVm(f, vmPid)
      const second = f.start()
      await expectExit(second, 2)
      expect(first.child.exitCode).toBeNull()
      expect(await processRunning(vmPid)).toBe(true)
      expect(Number(await readFile(join(f.jail, 'pid'), 'utf8'))).toBe(vmPid)
      await f.exitVm()
      await expectExit(first, 0)
    } finally {
      await f.close()
    }
  },
  15_000
)

linuxRoot(
  'a failed jailer preserves its exit status and removes the partial jail',
  async () => {
    const f = await fixture('fail')
    try {
      const started = f.start()
      await expectExit(started, 23)
      expect(existsSync(f.jail)).toBe(false)
      expect(existsSync(join(f.directory, 'vm.json'))).toBe(false)
    } finally {
      await f.close()
    }
  }
)

for (const mode of ['no-pid', 'no-socket'] as const) {
  linuxRoot(
    `a jailer that never supplies ${mode === 'no-pid' ? 'a VM pid' : 'an API socket'} fails and cleans up`,
    async () => {
      const f = await fixture(mode)
      try {
        const started = f.start()
        await expectExit(started, 2)
        expect(existsSync(f.jail)).toBe(false)
        if (mode === 'no-socket')
          expect(await processRunning(await f.pid('vm'))).toBe(false)
      } finally {
        await f.close()
      }
    },
    10_000
  )
}

linuxRoot(
  'a disk preparation failure never starts the jailer and leaves no jail',
  async () => {
    const f = await fixture()
    try {
      await rm(f.kernel)
      const started = f.start()
      await expectExit(started, 2)
      expect(existsSync(f.jail)).toBe(false)
      expect(existsSync(join(f.directory, 'launcher.json'))).toBe(false)
    } finally {
      await f.close()
    }
  }
)

linuxRoot(
  'arguments are validated before creating a jail or invoking a launcher',
  async () => {
    const f = await fixture()
    const replace = (flag: string, value: string) => {
      const args = [...f.args]
      args[args.indexOf(flag) + 1] = value
      return args
    }
    const invalid = [
      ['vm', 'unknown'],
      [...f.args, '--unknown', 'value'],
      [...f.args, '--id', 'duplicate'],
      [...f.args, '--unknown'],
      replace('--id', '../escape'),
      replace('--id', ''),
      replace('--id', 'x'.repeat(65)),
      replace('--kernel', 'relative-kernel'),
      replace('--chroot-base', `${f.directory}/../escape`),
      replace('--uid', '999'),
      replace('--gid', '0'),
      replace('--backend-gid', '0'),
      replace('--uid', 'not-a-number'),
      replace('--gid', '1000; false'),
    ]
    try {
      for (const args of invalid) {
        const result = f.start(args)
        await expectExit(result, 2)
        expect(await result.stderr).not.toBe('')
        expect(existsSync(f.chrootBase)).toBe(false)
        expect(existsSync(join(f.directory, 'launcher.json'))).toBe(false)
      }
    } finally {
      await f.close()
    }
  }
)
