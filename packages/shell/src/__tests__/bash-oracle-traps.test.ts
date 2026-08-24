import { mkdtemp, mkdir, rm, writeFile, chmod, symlink } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { oracle, runBash } from './bash-oracle-helpers'

/**
 * Extra GNU bash measurements for `docs/bash-behavior.md`: identity,
 * type/mode, cwd inode, spawn errno, and cd/pwd edges models use to
 * diagnose the machine. Same `PATH=/usr/bin:/bin` helper as
 * `bash-oracle.test.ts`.
 */

test.skipIf(!oracle.ok)('rg is a PATH file when present; command -v true is the builtin name', async () => {
  const result = await runBash(`
    type -P rg >/dev/null 2>&1; echo rg_p=$?
    type -t rg; echo rg_t=$?
    command -v true
  `)
  expect(result.stdout).toMatch(/^true$/m)
  if (result.stdout.includes('rg_p=0')) {
    expect(result.stdout).toMatch(/^file$/m)
    expect(result.stdout).toContain('rg_t=0')
  } else {
    expect(result.stdout).toContain('rg_p=1')
    expect(result.stdout).toContain('rg_t=1')
  }
})

test.skipIf(!oracle.ok)('$USER/$LOGNAME are env; whoami/id/$HOSTNAME/$UID are the process', async () => {
  const inherit = await runBash(`
    whoami
    id -un
    hostname
    printf 'HOSTNAME=%s\\n' "$HOSTNAME"
    printf 'USER=%s LOGNAME=%s\\n' "$USER" "$LOGNAME"
    printf 'UID=%s EUID=%s\\n' "$UID" "$EUID"
    id -u
  `)
  const lines = inherit.stdout.trim().split('\n')
  const who = lines[0]
  const idun = lines[1]
  const host = lines[2]
  expect(who).toBe(idun)
  expect(who.length).toBeGreaterThan(0)
  expect(host.length).toBeGreaterThan(0)
  expect(lines[3]).toBe(`HOSTNAME=${host}`)
  expect(lines[4]).toBe('USER= LOGNAME=')
  expect(lines[5]).toBe(`UID=${lines[6]} EUID=${lines[6]}`)

  const assigned = await runBash(
    'printf "USER=%s LOGNAME=%s\\n" "$USER" "$LOGNAME"; whoami',
    { env: { USER: 'alice', LOGNAME: 'bob' } },
  )
  expect(assigned.stdout).toContain('USER=alice LOGNAME=bob\n')
  expect(assigned.stdout).toContain(`${who}\n`)
  expect(assigned.stdout).not.toContain('alice\n')
})

test.skipIf(!oracle.ok)('ls -l owner, stat %U/%u, and id agree on files you create', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-owner-'))
  try {
    await writeFile(join(root, 'f'), 'x\n')
    const result = await runBash(`
      id -un
      id -gn
      id -u
      id -g
      ls -l f
      stat -c '%U %G %u %g %A %a' f
    `, { cwd: root })
    const [user, group, uid, gid, listing, st] = result.stdout.trim().split('\n')
    expect(listing).toMatch(new RegExp(`^-rw-r--r-- 1 ${user} ${group} `))
    expect(st).toBe(`${user} ${group} ${uid} ${gid} -rw-r--r-- 644`)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('test -O/-G/-c/-p/-r/-w/-u/-g/-k match kernel bits and ownership', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-testops-'))
  try {
    await writeFile(join(root, 'own'), 'x\n')
    await writeFile(join(root, 'setuid'), '')
    await writeFile(join(root, 'setgid'), '')
    await mkdir(join(root, 'stickyd'))
    const result = await runBash(`
      test -c /dev/null; echo c_null=$?
      test -c own; echo c_own=$?
      test -e /dev/null; echo e_null=$?
      test -O own; echo O_own=$?
      test -G own; echo G_own=$?
      test -O /dev/null; echo O_null=$?
      test -G /dev/null; echo G_null=$?
      chmod 444 own
      test -r own; echo r444=$?
      test -w own; echo w444=$?
      chmod 222 own
      test -r own; echo r222=$?
      test -w own; echo w222=$?
      chmod 4755 setuid
      chmod 2755 setgid
      chmod 1777 stickyd
      test -u setuid; echo u_setuid=$?
      test -g setgid; echo g_setgid=$?
      test -u setgid; echo u_setgid=$?
      test -k stickyd; echo k_sticky=$?
      ls -l setgid
      ls -ld stickyd
      mkfifo pipef
      test -p pipef; echo p_fifo=$?
      test -p own; echo p_own=$?
      ls -l pipef
    `, { cwd: root })
    expect(result.stdout).toContain('c_null=0')
    expect(result.stdout).toContain('c_own=1')
    expect(result.stdout).toContain('e_null=0')
    expect(result.stdout).toContain('O_own=0')
    expect(result.stdout).toContain('G_own=0')
    const uid = (await runBash('id -u')).stdout.trim()
    if (uid === '0') {
      expect(result.stdout).toContain('O_null=0')
      expect(result.stdout).toContain('G_null=0')
    } else {
      expect(result.stdout).toContain('O_null=1')
      expect(result.stdout).toContain('G_null=1')
    }
    expect(result.stdout).toContain('r444=0')
    expect(result.stdout).toContain('w444=1')
    expect(result.stdout).toContain('r222=1')
    expect(result.stdout).toContain('w222=0')
    expect(result.stdout).toContain('u_setuid=0')
    expect(result.stdout).toContain('g_setgid=0')
    expect(result.stdout).toContain('u_setgid=1')
    expect(result.stdout).toContain('k_sticky=0')
    expect(result.stdout).toMatch(/^-rwxr-sr-x .+ setgid$/m)
    expect(result.stdout).toMatch(/^drwxrwxrwt .+ stickyd$/m)
    expect(result.stdout).toContain('p_fifo=0')
    expect(result.stdout).toContain('p_own=1')
    expect(result.stdout).toMatch(/^prw-r--r-- .+ pipef$/m)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('dangling symlink, readlink, and GNU stat %F agree on type', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-types-'))
  try {
    await writeFile(join(root, 'target'), 'hello\n')
    await writeFile(join(root, 'empty'), '')
    await writeFile(join(root, 'script.sh'), '#!/bin/sh\necho x\n')
    await chmod(join(root, 'script.sh'), 0o755)
    await symlink('target', join(root, 'link'))
    await symlink('missing', join(root, 'dangling'))
    const result = await runBash(`
      readlink link; echo readlink_link=$?
      readlink dangling; echo readlink_dangling=$?
      readlink -f link
      readlink -f dangling; echo readlink_f_dangling=$?
      test -e dangling; echo e_dangling=$?
      test -L dangling; echo L_dangling=$?
      test -h dangling; echo h_dangling=$?
      test -f dangling; echo f_dangling=$?
      test -f link; echo f_link=$?
      ls -l dangling
      cat dangling; echo cat_dangling=$?
      stat -c %F empty
      stat -c %F target
      stat -c %F .
      stat -c %F link
      stat -c %F dangling
      mkfifo pipef
      stat -c %F pipef
      stat -c %F /dev/null
      stat -L -c %F link
      file -b empty
      file -b script.sh
    `, { cwd: root })
    expect(result.stdout).toContain('target\n')
    expect(result.stdout).toContain('readlink_link=0')
    expect(result.stdout).toContain('missing\n')
    expect(result.stdout).toContain('readlink_dangling=0')
    expect(result.stdout).toContain(join(root, 'target'))
    expect(result.stdout).toContain(join(root, 'missing'))
    expect(result.stdout).toContain('readlink_f_dangling=0')
    expect(result.stdout).toContain('e_dangling=1')
    expect(result.stdout).toContain('L_dangling=0')
    expect(result.stdout).toContain('h_dangling=0')
    expect(result.stdout).toContain('f_dangling=1')
    expect(result.stdout).toContain('f_link=0')
    expect(result.stdout).toMatch(/^lrwxrwxrwx .+ dangling -> missing$/m)
    expect(result.stdout).toContain('cat_dangling=1')
    expect(result.stderr).toMatch(/dangling: No such file or directory/)
    expect(result.stdout).toContain('regular empty file\n')
    expect(result.stdout).toContain('regular file\n')
    expect(result.stdout).toContain('directory\n')
    expect(result.stdout).toContain('symbolic link\n')
    expect(result.stdout).toContain('fifo\n')
    expect(result.stdout).toContain('character special file\n')
    expect(result.stdout).toMatch(/^empty$/m)
    expect(result.stdout).toContain('script')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('cd -P missing, cd to a file, HOME unset, logical vs physical ..', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cdtrap-'))
  try {
    await mkdir(join(root, 'real/sub'), { recursive: true })
    await symlink(join(root, 'real'), join(root, 'linkdir'))
    await writeFile(join(root, 'afile'), 'x\n')

    const missing = await runBash(`cd -P ${root}/nope`)
    expect(missing.exitCode).toBe(1)
    expect(missing.stderr).toContain(`cd: ${root}/nope: No such file or directory`)

    const file = await runBash(`cd ${join(root, 'afile')}`)
    expect(file.exitCode).toBe(1)
    expect(file.stderr).toContain(`cd: ${join(root, 'afile')}: Not a directory`)

    const nohome = await runBash('unset HOME; cd')
    expect(nohome.exitCode).toBe(1)
    expect(nohome.stderr).toContain('cd: HOME not set')

    const home = await runBash(`cd; printf 'PWD=%s\\n' "$PWD"`, { env: { HOME: root } })
    expect(home.exitCode).toBe(0)
    expect(home.stdout).toBe(`PWD=${root}\n`)

    const logical = await runBash(`
      cd "${join(root, 'linkdir/sub')}"
      printf 'PWD=%s\\n' "$PWD"
      cd ..
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(logical.stdout).toBe(
      `PWD=${join(root, 'linkdir/sub')}\nPWD=${join(root, 'linkdir')}\n`,
    )

    const physical = await runBash(`
      cd -P "${join(root, 'linkdir/sub')}"
      printf 'PWD=%s\\n' "$PWD"
      cd ..
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(physical.stdout).toBe(
      `PWD=${join(root, 'real/sub')}\nPWD=${join(root, 'real')}\n`,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('umask of a new file matches ls -l and stat %a', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-umask-'))
  try {
    const result = await runBash(`
      umask 077
      touch newf
      stat -c %a newf
      ls -l newf
    `, { cwd: root })
    expect(result.stdout).toMatch(/^600$/m)
    expect(result.stdout).toMatch(/^-rw------- .+ newf$/m)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('empty PATH: builtins run; PATH names fail 127 with No such file', async () => {
  const result = await runBash(`
    PATH=
    echo hi
    type -t echo
    true; echo true=$?
    type -t ls; echo type_ls=$?
    ls; echo ls=$?
    /bin/true; echo bintrue=$?
  `)
  expect(result.stdout).toContain('hi\n')
  expect(result.stdout).toContain('builtin')
  expect(result.stdout).toContain('true=0')
  expect(result.stdout).toContain('type_ls=1')
  expect(result.stdout).toContain('ls=127')
  expect(result.stderr).toMatch(/ls: No such file or directory/)
  expect(result.stderr).not.toContain('command not found')
  expect(result.stdout).toContain('bintrue=0')
})

test.skipIf(!oracle.ok)('PATH=. finds an executable in cwd', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-pathdot-'))
  try {
    await writeFile(join(root, 'localtool'), '#!/bin/sh\necho LOCAL\n')
    await chmod(join(root, 'localtool'), 0o755)
    const result = await runBash('PATH=.; localtool', { cwd: root })
    expect(result.exitCode).toBe(0)
    expect(result.stdout).toBe('LOCAL\n')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('assigning PATH clears the hash table', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-hashpath-'))
  try {
    await mkdir(join(root, 'a'))
    await mkdir(join(root, 'b'))
    await writeFile(join(root, 'a/oracletool'), '#!/bin/sh\necho FROM_A\n')
    await writeFile(join(root, 'b/oracletool'), '#!/bin/sh\necho FROM_B\n')
    await chmod(join(root, 'a/oracletool'), 0o755)
    await chmod(join(root, 'b/oracletool'), 0o755)
    const result = await runBash(`
      PATH="${join(root, 'a')}:${join(root, 'b')}:/usr/bin:/bin"
      oracletool
      hash -t oracletool
      PATH="${join(root, 'b')}:${join(root, 'a')}:/usr/bin:/bin"
      hash -t oracletool; echo hash_t=$?
      oracletool
    `)
    expect(result.stdout).toContain('FROM_A\n')
    expect(result.stdout).toContain(join(root, 'a/oracletool'))
    expect(result.stderr).toContain('hash: oracletool: not found')
    expect(result.stdout).toContain('hash_t=1')
    expect(result.stdout).toContain('FROM_B\n')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('command bypasses functions; missing names are not found', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-command-'))
  try {
    await mkdir(join(root, 'empty'))
    const result = await runBash(`
      cd empty
      ls() { echo FUNC; }
      command ls
      echo after_command
      command -v definitely_not_a_command_xyz; echo cv=$?
      type definitely_not_a_command_xyz; echo type=$?
    `, { cwd: root })
    expect(result.stdout).not.toContain('FUNC')
    expect(result.stdout).toContain('after_command')
    expect(result.stdout).toContain('cv=1')
    expect(result.stdout).toContain('type=1')
    expect(result.stderr).toContain('type: definitely_not_a_command_xyz: not found')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('script-file command-not-found names the file, not bash -c', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-script-'))
  try {
    const script = join(root, 's.sh')
    await writeFile(script, '#!/bin/bash\ndefinitely_not_a_command_xyz\n')
    await chmod(script, 0o755)
    const viaBash = await runBash(`bash "${script}"`)
    expect(viaBash.exitCode).toBe(127)
    expect(viaBash.stderr).toBe(`${script}: line 2: definitely_not_a_command_xyz: command not found\n`)

    const execd = await runBash(`"${script}"`)
    expect(execd.exitCode).toBe(127)
    expect(execd.stderr).toBe(`${script}: line 2: definitely_not_a_command_xyz: command not found\n`)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('ENOTDIR, missing source, noclobber, and missing redirect target stay distinct', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-errclass-'))
  try {
    await writeFile(join(root, 'afile'), 'x\n')
    const enotdir = await runBash('cat afile/foo', { cwd: root })
    expect(enotdir.exitCode).toBe(1)
    expect(enotdir.stderr).toContain('afile/foo: Not a directory')

    const source = await runBash('. /no/such/sourcefile')
    expect(source.exitCode).toBe(1)
    expect(source.stderr).toContain('/no/such/sourcefile: No such file or directory')

    const noclobber = await runBash(`
      echo first > ncf
      set -o noclobber
      echo second > ncf
      echo noclobber=$?
      cat ncf
    `, { cwd: root })
    expect(noclobber.stdout).toContain('noclobber=1')
    expect(noclobber.stdout).toContain('first\n')
    expect(noclobber.stderr).toContain('cannot overwrite existing file')

    const redir = await runBash('echo x > missingdir/f', { cwd: root })
    expect(redir.exitCode).toBe(1)
    expect(redir.stderr).toMatch(/missingdir\/f: No such file or directory/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('deleted cwd: /bin/pwd -L, PATH names, and background jobs still run', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cwdmore-'))
  const gone = join(root, 'gone')
  const abs = join(root, 'absfile')
  try {
    await mkdir(gone)
    await writeFile(abs, 'findme\n')
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      /bin/pwd -L; echo bin_pwd_L=$?
      true; echo true=$?
      false; echo false=$?
      chmod 600 "${abs}"; echo chmod=$?
      stat -c %a "${abs}"; echo stat=$?
      grep findme "${abs}"; echo grep=$?
      id -un >/dev/null; echo id=$?
      whoami >/dev/null; echo whoami=$?
      hostname >/dev/null; echo hostname=$?
      /bin/echo from_bg &
      wait
      echo wait=$?
    `)
    expect(result.stdout).toContain('bin_pwd_L=1')
    expect(result.stderr).toContain("/bin/pwd: couldn't find directory entry in '..' with matching i-node")
    expect(result.stdout).toContain('true=0')
    expect(result.stdout).toContain('false=1')
    expect(result.stdout).toContain('chmod=0')
    expect(result.stdout).toContain('600\n')
    expect(result.stdout).toContain('stat=0')
    expect(result.stdout).toContain('findme\n')
    expect(result.stdout).toContain('grep=0')
    expect(result.stdout).toContain('id=0')
    expect(result.stdout).toContain('whoami=0')
    expect(result.stdout).toContain('hostname=0')
    expect(result.stdout).toContain('from_bg\n')
    expect(result.stdout).toContain('wait=0')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('pwd -P after deleted cwd then cd .. sets PWD to ..', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-pwdpthen-'))
  const gone = join(root, 'gone')
  try {
    await mkdir(gone)
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      pwd -P
      cd ..
      echo cd_dotdot=$?
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(result.stderr).toContain('pwd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory')
    expect(result.stderr).toContain('chdir: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory')
    expect(result.stdout).toContain('cd_dotdot=0')
    expect(result.stdout).toContain('PWD=..\n')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test.skipIf(!oracle.ok)('cd -P . after deleted cwd appends /. like cd .', async () => {
  const root = await mkdtemp(join(tmpdir(), 'bash-oracle-cdpdot-'))
  const gone = join(root, 'gone')
  try {
    await mkdir(gone)
    const result = await runBash(`
      set +e
      cd "${gone}"
      rm -rf "${gone}"
      cd -P .
      echo cdp_dot=$?
      printf 'PWD=%s\\n' "$PWD"
    `)
    expect(result.stdout).toContain('cdp_dot=0')
    expect(result.stdout).toContain(`PWD=${gone}/.\n`)
    expect(result.stderr).toContain('cd: error retrieving current directory: getcwd: cannot access parent directories: No such file or directory')
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
