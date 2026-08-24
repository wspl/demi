import { mkdtemp, mkdir, rm, writeFile, chmod, symlink, link } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'

/**
 * GNU bash on Linux is the oracle for `docs/bash-behavior.md`.
 * Every bash-column claim in that document is asserted here with
 * `bash --norc --noprofile` so nobody edits the table from memory.
 */
const BASH = ['bash', '--norc', '--noprofile'] as const

const oracle = await probeOracle()

test('oracle is GNU bash on Linux', () => {
  expect(oracle.uname).toBe('Linux')
  expect(oracle.bashVersion).toContain('GNU bash')
})

test.skipIf(!oracle.ok)('type -t: builtins vs PATH files', async () => {
  const result = await runBash(`
    for n in cd test '[' export set source . pwd echo printf; do
      printf '%s %s\\n' "\$n" "\$(type -t "\$n")"
    done
    for n in ls cat chmod stat mkdir rm cp mv which file whoami hostname grep find; do
      printf '%s %s %s\\n' "\$n" "\$(type -t "\$n")" "\$(type -P "\$n")"
    done
  `)
  expect(result.exitCode).toBe(0)
  expect(result.stdout).toContain('cd builtin')
  expect(result.stdout).toContain('test builtin')
  expect(result.stdout).toContain('[ builtin')
  expect(result.stdout).toContain('pwd builtin')
  expect(result.stdout).toContain('echo builtin')
  expect(result.stdout).toContain('printf builtin')
  expect(result.stdout).toMatch(/^ls file \/.+\/ls$/m)
  expect(result.stdout).toMatch(/^chmod file \/.+\/chmod$/m)
  expect(result.stdout).toMatch(/^whoami file \/.+\/whoami$/m)
  expect(result.stdout).toMatch(/^grep file \/.+\/grep$/m)
})

test.skipIf(!oracle.ok)('functions override builtins; posix forbids special-builtin functions', async () => {
  const def = await runBash('cd() { echo FUNC-CD; }; cd /tmp; printf PWD=%s\\n "$PWD"')
  expect(def.stdout).toMatch(/^FUNC-CD$/m)
  expect(def.stdout).not.toContain('PWD=/tmp')

  const posix = await runBash('set -o posix; export() { echo FUNC; }; echo still-here')
  expect(posix.stderr).toContain('is a special builtin')
  expect(posix.stdout).not.toContain('still-here')
})

test.skipIf(!oracle.ok)('children receive exported variables only', async () => {
  const result = await runBash(`
    NOT_EXPORTED=secret
    export EXPORTED=visible
    /usr/bin/printenv NOT_EXPORTED; echo unexported=$?
    /usr/bin/printenv EXPORTED; echo exported=$?
  `)
  expect(result.stdout).toContain('unexported=1')
  expect(result.stdout).toContain('visible')
  expect(result.stdout).toContain('exported=0')
})

test.skipIf(!oracle.ok)('exec failures: not found, permission, directory, bad shebang', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-exec-'))
  try {
    await writeFile(join(root, 'noexec'), '#!/bin/sh\necho NO\n')
    await chmod(join(root, 'noexec'), 0o644)
    await mkdir(join(root, 'adir'))
    await writeFile(join(root, 'badshebang'), '#!/no/such/interpreter\necho NO\n')
    await chmod(join(root, 'badshebang'), 0o755)

    const missing = await runBash('definitely_not_a_command_xyz')
    expect(missing.exitCode).toBe(127)
    expect(missing.stderr).toMatch(/definitely_not_a_command_xyz: command not found\n$/)

    const noexec = await runBash(`"${join(root, 'noexec')}"`)
    expect(noexec.exitCode).toBe(126)
    expect(noexec.stderr).toContain('Permission denied')

    const dir = await runBash(`"${join(root, 'adir')}"`)
    expect(dir.exitCode).toBe(126)
    expect(dir.stderr).toContain('Is a directory')

    const shebang = await runBash(`"${join(root, 'badshebang')}"`)
    expect(shebang.exitCode).toBe(127)
    expect(shebang.stderr).toContain('cannot execute: required file not found')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('ls/stat/test/find/which agree on mode bits', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-meta-'))
  try {
    const file644 = join(root, 'file644')
    const file755 = join(root, 'file755')
    const file001 = join(root, 'file001')
    await writeFile(file644, 'hello\n')
    await writeFile(file755, 'hello\n')
    await writeFile(file001, 'hello\n')
    await chmod(file644, 0o644)
    await chmod(file755, 0o755)
    await chmod(file001, 0o001)
    await symlink('file755', join(root, 'link'))
    await link(file644, join(root, 'hard'))
    await writeFile(join(root, 'setuid'), '')
    await writeFile(join(root, 'sticky'), '')
    const special = await runBash('chmod 4755 setuid; chmod 1755 sticky; ls -l setuid sticky', { cwd: root })
    expect(special.exitCode).toBe(0)
    expect(special.stdout).toMatch(/^-rwsr-xr-x .+ setuid$/m)
    expect(special.stdout).toMatch(/^-rwxr-xr-t .+ sticky$/m)

    const listing = await runBash('ls -l file755 file644 link', { cwd: root })
    expect(listing.exitCode).toBe(0)
    expect(listing.stdout).toMatch(/^-rwxr-xr-x .+ file755$/m)
    expect(listing.stdout).toMatch(/^-rw-r--r-- .+ file644$/m)
    expect(listing.stdout).toMatch(/^lrwxrwxrwx .+ link -> file755$/m)

    const st = await runBash('stat -c %A file755; stat -c %A file644; stat -c %A link; stat -c %h file644', { cwd: root })
    expect(st.stdout).toBe('-rwxr-xr-x\n-rw-r--r--\nlrwxrwxrwx\n2\n')

    const tests = await runBash(`
      test -x file755; echo x755=$?
      test -x file644; echo x644=$?
      test -x file001; echo x001=$?
      test -L link; echo Llink=$?
      test -f link; echo flink=$?
      test file644 -ef hard; echo ef_hard=$?
      test file644 -ef file755; echo ef_other=$?
      test link -ef file755; echo ef_link=$?
    `, { cwd: root })
    expect(tests.stdout).toContain('x755=0')
    expect(tests.stdout).toContain('x644=1')
    expect(tests.stdout).toContain('x001=1')
    expect(tests.stdout).toContain('Llink=0')
    expect(tests.stdout).toContain('flink=0')
    expect(tests.stdout).toContain('ef_hard=0')
    expect(tests.stdout).toContain('ef_other=1')
    expect(tests.stdout).toContain('ef_link=0')

    const found = await runBash('find . -perm /111 -printf "%f\\n" | sort', { cwd: root })
    expect(found.stdout).toContain('file755\n')
    expect(found.stdout).not.toContain('file644\n')
    expect(found.stdout).toContain('link\n')

    const classify = await runBash('ls -F file755 file644 link', { cwd: root })
    expect(classify.stdout).toContain('file755*\n')
    expect(classify.stdout).toContain('file644\n')
    expect(classify.stdout).toContain('link@\n')

    const dirList = await runBash('ls -l', { cwd: root })
    const total = Number(dirList.stdout.match(/^total (\d+)$/m)?.[1])
    const names = dirList.stdout.trim().split('\n').filter((line) => !line.startsWith('total ')).length
    expect(total).toBeGreaterThan(0)
    expect(total).not.toBe(names)

    await writeFile(join(root, 'only644'), '#!/bin/sh\necho RUN\n')
    await chmod(join(root, 'only644'), 0o644)
    await writeFile(join(root, 'ok755'), '#!/bin/sh\necho RUN\n')
    await chmod(join(root, 'ok755'), 0o755)

    const which644 = await runBash('which only644; echo which=$?; type -P only644; echo typeP=$?', {
      env: { PATH: `${root}:/usr/bin:/bin` },
    })
    expect(which644.stdout).toContain('which=1')
    expect(which644.stdout).toContain(`${join(root, 'only644')}`)
    expect(which644.stdout).toContain('typeP=0')

    const which755 = await runBash('which ok755; echo which=$?', {
      env: { PATH: `${root}:/usr/bin:/bin` },
    })
    expect(which755.stdout).toContain(join(root, 'ok755'))
    expect(which755.stdout).toContain('which=0')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('pwd -L is logical; pwd -P and GNU pwd are physical', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-pwd-'))
  try {
    await mkdir(join(root, 'realdir'))
    await symlink(join(root, 'realdir'), join(root, 'linkdir'))
    const result = await runBash(`
      cd "${join(root, 'linkdir')}"
      printf 'PWD=%s\\n' "$PWD"
      pwd
      pwd -L
      pwd -P
      /bin/pwd
      /bin/pwd -L
      /bin/pwd -P
    `)
    const link = join(root, 'linkdir')
    const real = join(root, 'realdir')
    expect(result.stdout).toBe(
      `PWD=${link}\n${link}\n${link}\n${real}\n${real}\n${link}\n${real}\n`,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('cd missing path; cd -; CDPATH', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cd-'))
  try {
    await mkdir(join(root, 'here'))
    await mkdir(join(root, 'cdpath/target'), { recursive: true })

    const missing = await runBash(`cd "${root}/nope"`)
    expect(missing.exitCode).toBe(1)
    expect(missing.stderr).toContain(`cd: ${root}/nope: No such file or directory`)

    const dash = await runBash(`cd "${join(root, 'here')}"; cd /tmp; cd -`)
    expect(dash.exitCode).toBe(0)
    expect(dash.stdout).toBe(`${join(root, 'here')}\n`)

    const cdpath = await runBash(`CDPATH="${join(root, 'cdpath')}"; cd target; printf 'PWD=%s\\n' "$PWD"`)
    expect(cdpath.stdout).toBe(`${join(root, 'cdpath/target')}\nPWD=${join(root, 'cdpath/target')}\n`)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('whoami, $UID, and id -u are the same principal', async () => {
  const result = await runBash(`
    whoami
    id -un
    printf 'UID=%s EUID=%s\\n' "$UID" "$EUID"
    id -u
  `)
  const [who, idun, uidLine, idu] = result.stdout.trim().split('\n')
  expect(who).toBe(idun)
  expect(uidLine).toBe(`UID=${idu} EUID=${idu}`)
  expect(result.exitCode).toBe(0)
})

test.skipIf(!oracle.ok)('deleted cwd keeps the inode; path-based create fails', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cwd-'))
  const gone = join(root, 'gone')
  try {
    await mkdir(gone)
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      pwd -L; echo pwd_L=$?
      printf 'PWD=%s\\n' "$PWD"
      pwd -P; echo pwd_P=$?
      /bin/pwd; echo bin_pwd=$?
      /bin/echo ok; echo echo_ok=$?
      ls .; echo ls_dot=$?
      ls -ld .
      echo x > relfile; echo relwrite=$?
      /bin/true; echo true=$?
    `)

    expect(result.stdout).toContain(`${gone}\n`)
    expect(result.stdout).toContain('pwd_L=0')
    expect(result.stdout).toContain(`PWD=${gone}`)
    expect(result.stdout).toContain('pwd_P=1')
    expect(result.stderr).toContain('pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory')
    expect(result.stdout).toContain('bin_pwd=1')
    expect(result.stderr).toContain("/bin/pwd: couldn't find directory entry in '..' with matching i-node")
    expect(result.stdout).toContain('ok\n')
    expect(result.stdout).toContain('echo_ok=0')
    expect(result.stdout).toContain('ls_dot=0')
    expect(result.stdout).toMatch(/^drwxr-xr-x 0 .+ \.$/m)
    expect(result.stdout).toContain('relwrite=1')
    expect(result.stderr).toMatch(/relfile: No such file or directory/)
    expect(result.stdout).toContain('true=0')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('cd . after deleted cwd exits 0 and appends /. to PWD', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cddot-'))
  const gone = join(root, 'gone')
  try {
    await mkdir(gone)
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      cd .
      echo cd_dot=$?
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(result.stdout).toContain('cd_dot=0')
    expect(result.stdout).toContain(`PWD=${gone}/.\n`)
    expect(result.stderr).toContain('cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('cd .. immediately after deleted cwd lands in the parent path', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cddotdot-'))
  const gone = join(root, 'gone')
  try {
    await mkdir(gone)
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      cd ..
      echo cd_dotdot=$?
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(result.stdout).toContain('cd_dotdot=0')
    expect(result.stdout).toContain(`PWD=${root}\n`)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('hash remembers a pathname; hash -r re-searches PATH', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-hash-'))
  try {
    await mkdir(join(root, 'a'))
    await mkdir(join(root, 'b'))
    await writeFile(join(root, 'a/oracletool'), '#!/bin/sh\necho FROM_A\n')
    await writeFile(join(root, 'b/oracletool'), '#!/bin/sh\necho FROM_B\n')
    await chmod(join(root, 'a/oracletool'), 0o755)
    await chmod(join(root, 'b/oracletool'), 0o755)
    const result = await runBash(`
      set +e
      PATH="${join(root, 'a')}:${join(root, 'b')}:/usr/bin:/bin"
      oracletool
      hash -t oracletool
      rm "${join(root, 'a/oracletool')}"
      oracletool
      echo hashed_missing=$?
      hash -r
      oracletool
      echo after_hash_r=$?
    `)
    expect(result.stdout).toContain('FROM_A\n')
    expect(result.stdout).toContain(join(root, 'a/oracletool'))
    expect(result.stdout).toContain('hashed_missing=127')
    expect(result.stderr).toContain(`${join(root, 'a/oracletool')}: No such file or directory`)
    expect(result.stdout).toContain('FROM_B\n')
    expect(result.stdout).toContain('after_hash_r=0')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('type pwd is builtin; type ls and command -v ls are the PATH file', async () => {
  const result = await runBash('type pwd; type ls; command -v pwd; command -v ls; type /bin/ls')
  expect(result.stdout).toContain('pwd is a shell builtin')
  expect(result.stdout).toMatch(/^ls is \/.+\/ls$/m)
  expect(result.stdout).toMatch(/^pwd$/m)
  expect(result.stdout).toMatch(/^\/.+\/ls$/m)
  expect(result.stdout).toContain('/bin/ls is /bin/ls')
})

async function probeOracle(): Promise<{ ok: boolean; uname: string; bashVersion: string }> {
  const uname = await runCommand(['uname', '-s'])
  const bashVersion = await runCommand(['bash', '--version'])
  return {
    ok: uname.stdout.trim() === 'Linux' && bashVersion.stdout.includes('GNU bash'),
    uname: uname.stdout.trim(),
    bashVersion: bashVersion.stdout.split('\n')[0] ?? '',
  }
}

async function runBash(
  script: string,
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return runCommand([...BASH, '-c', script], options)
}

async function runCommand(
  argv: string[],
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  const proc = Bun.spawn(argv, {
    cwd: options.cwd,
    env: {
      PATH: '/usr/bin:/bin',
      HOME: options.cwd ?? tmpdir(),
      LC_ALL: 'C',
      LANG: 'C',
      ...options.env,
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const stdout = await new Response(proc.stdout).text()
  const stderr = await new Response(proc.stderr).text()
  const exitCode = await proc.exited
  return { stdout, stderr, exitCode }
}
