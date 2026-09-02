import { chmod, mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir, userInfo } from 'node:os'
import { expect, test } from 'bun:test'
import { BashEnvironment } from '../bash'
import { LocalHost } from '@demicodes/host-local'
import { oracle } from './bash-oracle-helpers'

function createEnv(root: string, extraEnv: Record<string, string> = {}) {
  return new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: '/usr/bin:/bin', HOME: root, ...extraEnv },
  })
}

test.skipIf(!oracle.ok)('Host-backed ls -l reports real mode, not portable 644', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-ls-'))
  const file = join(root, 'exec.sh')
  await writeFile(file, '#!/bin/sh\n')
  await chmod(file, 0o755)
  const env = createEnv(root)
  const result = await env.exec({ script: 'ls -l exec.sh', timeoutMs: 10_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toMatch(/^-rwxr-xr-x\b/)
  expect(result.stdout.delta).not.toContain('rw-r--r--')
  expect(result.audit.map((event) => event.kind)).toContain('system-command')
})

test.skipIf(!oracle.ok)('Host-backed whoami, id, $UID, and ls owner agree', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-id-'))
  await writeFile(join(root, 'f'), 'x\n')
  const env = createEnv(root)
  const result = await env.exec({
    script: 'whoami; id -un; echo UID=$UID; ls -l f | awk \'{print $3}\'',
    timeoutMs: 10_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  const lines = result.stdout.delta.trim().split('\n')
  expect(lines[0]).toBe(userInfo().username)
  expect(lines[1]).toBe(lines[0])
  expect(lines[2]).toBe(`UID=${userInfo().uid}`)
  expect(lines[3]).toBe(lines[0])
})

test.skipIf(!oracle.ok)('deleted cwd still runs PATH binaries; grep does not fall back to portable', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-cwd-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  await writeFile(join(dir, 'notes.txt'), 'alpha\n')
  const env = createEnv(root)
  const result = await env.exec({
    script: `cd gone && rm -rf "$PWD" && /bin/echo ok && grep -n alpha notes.txt; printf EXIT:$?`,
    timeoutMs: 10_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toContain('ok')
  expect(result.audit.filter((event) => event.kind === 'portable-command' && event.name === 'grep')).toEqual([])
  expect(result.audit.map((event) => ('name' in event ? event.name : ''))).toContain('grep')
})

test.skipIf(!oracle.ok)('missing name is command-not-found, not a portable fallback', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-miss-'))
  const env = createEnv(root)
  const result = await env.exec({ script: 'definitely_not_a_command_xyz', timeoutMs: 5_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(127)
  expect(result.stderr.delta).toContain('command not found')
  expect(result.stderr.delta).not.toMatch(/posix_spawn/i)
})

test.skipIf(!oracle.ok)('type ls describes the PATH file dispatch will run', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-type-'))
  const env = createEnv(root)
  const result = await env.exec({ script: 'type ls; type -t ls; command -v ls', timeoutMs: 5_000 })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toMatch(/^ls is \/.+\/ls$/m)
  expect(result.stdout.delta).toMatch(/^file$/m)
  expect(result.stdout.delta).toMatch(/\/ls$/m)
  expect(result.stdout.delta).not.toContain('registered command')
})

test.skipIf(!oracle.ok)('children receive exported variables only', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-env-'))
  const env = createEnv(root)
  const result = await env.exec({
    script: `
      NOT_EXPORTED=secret
      export EXPORTED=visible
      /usr/bin/printenv NOT_EXPORTED; echo unexported=$?
      /usr/bin/printenv EXPORTED; echo exported=$?
    `,
    timeoutMs: 5_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toContain('unexported=1')
  expect(result.stdout.delta).toContain('visible')
  expect(result.stdout.delta).toContain('exported=0')
})

test.skipIf(!oracle.ok)('Host-backed cd .. after deleted cwd lands in the parent', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-cddotdot-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  const env = createEnv(root)
  const result = await env.exec({
    script: `cd gone && rm -rf "$PWD" && cd .. && printf 'PWD=%s\\n' "$PWD" && echo exit:$?`,
    timeoutMs: 10_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toContain(`PWD=${root}`)
  expect(result.stdout.delta).toContain('exit:0')
})

test.skipIf(!oracle.ok)('Host-backed pwd -P fails after deleted cwd; cd with HOME unset fails', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-pwd-home-'))
  const dir = join(root, 'gone')
  await mkdir(dir)
  const pwdEnv = createEnv(root)
  const gone = await pwdEnv.exec({
    script: `cd gone && rm -rf "$PWD" && pwd -P; echo pwd_P:$?`,
    timeoutMs: 10_000,
  })
  expect(gone.status).toBe('exited')
  if (gone.status !== 'exited') throw new Error('expected exited')
  expect(gone.stdout.delta).toContain('pwd_P:1')
  expect(gone.stderr.delta).toContain(
    'pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory',
  )

  const nohome = await new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: '/usr/bin:/bin' },
  }).exec({
    script: 'unset HOME; cd; echo cd:$?',
    timeoutMs: 5_000,
  })
  expect(nohome.status).toBe('exited')
  if (nohome.status !== 'exited') throw new Error('expected exited')
  expect(nohome.stdout.delta).toContain('cd:1')
  expect(nohome.stderr.delta).toContain('HOME not set')
})

test.skipIf(!oracle.ok)('Host-backed test -c / -O / -x match the Host inode', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-host-test-'))
  const file = join(root, 'exec.sh')
  await writeFile(file, '#!/bin/sh\n')
  await chmod(file, 0o755)
  const owned = join(root, 'owned')
  await writeFile(owned, 'x\n')
  await chmod(owned, 0o001)
  const env = createEnv(root)
  const result = await env.exec({
    script: `
      [ -c /dev/null ]; echo char:$?
      [ -O /dev/null ]; echo nullown:$?
      [ -O owned ]; echo fileown:$?
      [ -x exec.sh ]; echo exec:$?
      [ -x owned ]; echo ownerx:$?
    `,
    timeoutMs: 5_000,
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited')
  expect(result.stdout.delta).toContain('char:0')
  expect(result.stdout.delta).toContain('nullown:1')
  expect(result.stdout.delta).toContain('fileown:0')
  expect(result.stdout.delta).toContain('exec:0')
  expect(result.stdout.delta).toContain('ownerx:1')
})
