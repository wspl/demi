import { describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { runTinybash, type ShellState } from '../index'
import { stubRoots } from '../testing'

// `tar` as a builtin (`tinybash.md` § Builtins): archives round-trip with
// mode and mtime, members in name order, `-z` through the platform's
// gzip, `--strip-components`, GNU's path rules, links refused as the third
// run-time failure that is not an upgrade, and archives the system tar can
// read — the format is GNU's.

function world() {
  const dir = mkdtempSync(join(tmpdir(), 'tinybash-tar-'))
  const home = join(dir, 'home')
  mkdirSync(home)
  const host = new LocalHost(home, { storeRoot: join(dir, 'store') })
  const state: ShellState = { cwd: home, home, vars: { HOME: home } }
  const { roots, dispatch } = stubRoots({ demi: {} })
  let out = ''
  let err = ''
  const run = async (script: string) => {
    out = ''
    err = ''
    const result = await runTinybash({
      script,
      roots,
      namespace: [home],
      dispatch,
      fs: host.fs,
      state,
      io: {
        stdout: (d) => void (out += typeof d === 'string' ? d : new TextDecoder().decode(d)),
        stderr: (d) => void (err += typeof d === 'string' ? d : new TextDecoder().decode(d)),
      },
      identity: { user: 'demi', group: 'demi' },
    })
    return { exit: result.kind === 'ran' ? result.exitCode : 'outside', stdout: out, stderr: err }
  }
  const seed = () => {
    mkdirSync(join(home, 'src/lib'), { recursive: true })
    writeFileSync(join(home, 'src/main.ts'), 'export const main = 1\n')
    writeFileSync(join(home, 'src/lib/deep.ts'), 'deep\n')
    writeFileSync(join(home, 'src/run.sh'), '#!/bin/sh\necho hi\n', { mode: 0o755 })
    writeFileSync(join(home, 'notes.txt'), 'alpha\nbeta\n')
    const then = new Date('2024-01-15T12:00:00Z')
    for (const path of ['src/main.ts', 'src/lib/deep.ts', 'src/run.sh', 'notes.txt', 'src/lib', 'src']) {
      const full = join(home, path)
      require('node:fs').utimesSync(full, then, then)
    }
  }
  return { home, run, seed, dispose: () => rmSync(dir, { recursive: true, force: true }) }
}

describe('tar', () => {
  test('c then x round-trips a tree with modes and mtimes; members are listed in name order', async () => {
    const w = world()
    try {
      w.seed()
      expect(await w.run('tar c src notes.txt > a.tar && tar t < a.tar')).toEqual({
        exit: 0,
        stdout: 'src/\nsrc/lib/\nsrc/lib/deep.ts\nsrc/main.ts\nsrc/run.sh\nnotes.txt\n',
        stderr: '',
      })
      // GNU pads the archive to its 10 KiB record.
      expect(statSync(join(w.home, 'a.tar')).size % 10240).toBe(0)
      expect((await w.run('mkdir out && tar x -C out < a.tar && cat out/src/lib/deep.ts out/notes.txt')).stdout).toBe('deep\nalpha\nbeta\n')
      expect(statSync(join(w.home, 'out/src/run.sh')).mode & 0o777).toBe(0o755)
      expect(statSync(join(w.home, 'out/src/main.ts')).mode & 0o777).toBe(0o644)
      expect(statSync(join(w.home, 'out/src/main.ts')).mtime.toISOString()).toBe('2024-01-15T12:00:00.000Z')
      expect(statSync(join(w.home, 'out/src/lib')).mtime.toISOString()).toBe('2024-01-15T12:00:00.000Z')
      // The `-f` spelling, old-style letters, `-v`, and a pipe between two tars.
      expect((await w.run('tar cvf b.tar -C src lib')).stdout).toBe('lib/\nlib/deep.ts\n')
      expect((await w.run('tar tf b.tar')).stdout).toBe('lib/\nlib/deep.ts\n')
      expect((await w.run('mkdir piped && tar c -C src . | tar x -C piped && cat piped/main.ts')).stdout).toBe('export const main = 1\n')
      expect((await w.run('tar c -C src . | tar t')).stdout).toBe('./\n./lib/\n./lib/deep.ts\n./main.ts\n./run.sh\n')
    } finally {
      w.dispose()
    }
  })

  test('-z is gzip both ways; --strip-components drops leading components; operands select members', async () => {
    const w = world()
    try {
      w.seed()
      expect((await w.run('tar czf s.tgz src && tar tzf s.tgz')).stdout).toBe('src/\nsrc/lib/\nsrc/lib/deep.ts\nsrc/main.ts\nsrc/run.sh\n')
      const header = readFileSync(join(w.home, 's.tgz'))
      expect([header[0], header[1]]).toEqual([0x1f, 0x8b])
      expect((await w.run('mkdir flat && tar xzf s.tgz -C flat --strip-components=1 && ls flat')).stdout).toBe('lib\nmain.ts\nrun.sh\n')
      // Without `-z` a gzip stream is not an archive.
      expect((await w.run('mkdir some && tar xf s.tgz -C some src/lib')).exit).toBe(2)
      expect((await w.run('tar xzf s.tgz -C some src/lib && ls some some/src')).stdout).toBe('some:\nsrc\n\nsome/src:\nlib\n')
      expect((await w.run('tar tz < s.tgz src/main.ts')).stdout).toBe('src/main.ts\n')
    } finally {
      w.dispose()
    }
  })

  test('the errors GNU prints: a missing operand, an empty archive, a bad archive, a member with .., a link', async () => {
    const w = world()
    try {
      w.seed()
      // GNU still writes the archive's end when every member failed: one empty record.
      const missing = await w.run('tar c nope')
      expect([missing.exit, missing.stderr]).toEqual([2, 'tar: nope: Cannot stat: No such file or directory\ntar: Exiting with failure status due to previous errors\n'])
      expect(missing.stdout).toBe('\0'.repeat(10240))
      expect(await w.run('tar c')).toEqual({ exit: 2, stdout: '', stderr: "tar: Cowardly refusing to create an empty archive\nTry 'tar --help' or 'tar --usage' for more information.\n" })
      // A first word that is not options (`tar notes.txt`) is GNU's "invalid option": outside, a machine reports it.
      expect((await w.run('tar notes.txt')).exit).toBe('outside')
      expect((await w.run('tar -f a.tar notes.txt')).stderr).toContain("You must specify one of the '-Acdtrux'")
      expect(await w.run('tar x < notes.txt')).toEqual({
        exit: 2,
        stdout: '',
        stderr: 'tar: This does not look like a tar archive\ntar: Exiting with failure status due to previous errors\n',
      })
      // An archive with a `..` member and a symlink, as another host's tar could send: the rest is extracted, the exit is 2.
      writeFileSync(join(w.home, 'evil.tar'), evilArchive())
      expect(await w.run('mkdir in && tar x -C in < evil.tar')).toEqual({
        exit: 2,
        stdout: '',
        stderr: "tar: Removing leading `a/../' from member names\ntar: a/../escape.txt: Member name contains '..'\ntar: link.txt: Cannot create symlink to 'ok.txt': Operation not permitted\ntar: Exiting with failure status due to previous errors\n",
      })
      expect((await w.run('cat in/ok.txt')).stdout).toBe('fine\n')
    } finally {
      w.dispose()
    }
  })

  test('the system tar reads what tinybash writes, and tinybash reads what it writes; leading / is stripped', async () => {
    const w = world()
    try {
      w.seed()
      await w.run('tar cf mine.tar src notes.txt')
      const listed = Bun.spawnSync(['tar', 'tf', join(w.home, 'mine.tar')])
      expect(listed.exitCode).toBe(0)
      expect(listed.stdout.toString()).toBe('src/\nsrc/lib/\nsrc/lib/deep.ts\nsrc/main.ts\nsrc/run.sh\nnotes.txt\n')
      const theirs = Bun.spawnSync(['tar', 'cf', join(w.home, 'theirs.tar'), '-C', join(w.home, 'src'), 'main.ts', 'lib'])
      expect(theirs.exitCode).toBe(0)
      expect((await w.run('mkdir back && tar xf theirs.tar -C back && cat back/main.ts back/lib/deep.ts')).stdout).toBe('export const main = 1\ndeep\n')
      const absolute = await w.run(`tar c ${w.home}/notes.txt | tar t`)
      expect(absolute.stderr).toBe("tar: Removing leading `/' from member names\n")
      expect(absolute.stdout).toBe(`${w.home.replace(/^\/+/, '')}/notes.txt\n`)
    } finally {
      w.dispose()
    }
  })

  test('flags outside the whitelist and paths outside the namespace are outside the subset, before anything runs', async () => {
    const w = world()
    try {
      w.seed()
      for (const script of ['tar cjf x.tbz src', 'tar --exclude=x -c src', 'tar c -C /etc passwd', 'tar xf /var/x.tar', 'tar c src -f /var/a.tar', 'tar -c --strip-components=x src']) {
        expect((await w.run(script)).exit, script).toBe('outside')
      }
      expect(statSync(join(w.home, 'src')).isDirectory()).toBe(true)
    } finally {
      w.dispose()
    }
  })
})

/** A ustar archive by hand: `ok.txt`, then `a/../escape.txt`, then a symlink `link.txt -> ok.txt`. */
function evilArchive(): Uint8Array {
  const blocks: Uint8Array[] = []
  const header = (name: string, type: string, size: number, link = '') => {
    const block = new Uint8Array(512)
    const put = (offset: number, text: string) => block.set(new TextEncoder().encode(text), offset)
    put(0, name)
    put(100, '0000644\0')
    put(108, '0001000\0')
    put(116, '0001000\0')
    put(124, `${size.toString(8).padStart(11, '0')}\0`)
    put(136, '00000000000\0')
    put(156, type)
    put(157, link)
    put(257, 'ustar\0')
    put(263, '00')
    put(148, '        ')
    let sum = 0
    for (const byte of block) sum += byte
    put(148, `${sum.toString(8).padStart(6, '0')}\0 `)
    blocks.push(block)
    if (size > 0) {
      const data = new Uint8Array(Math.ceil(size / 512) * 512)
      data.set(new TextEncoder().encode('fine\n'))
      blocks.push(data)
    }
  }
  header('ok.txt', '0', 5)
  header('a/../escape.txt', '0', 5)
  header('link.txt', '2', 0, 'ok.txt')
  blocks.push(new Uint8Array(1024))
  const total = blocks.reduce((n, b) => n + b.byteLength, 0)
  const out = new Uint8Array(Math.ceil(total / 10240) * 10240)
  let at = 0
  for (const block of blocks) {
    out.set(block, at)
    at += block.byteLength
  }
  return out
}
