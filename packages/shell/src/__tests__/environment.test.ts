import { access, mkdir, mkdtemp, readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { z } from 'zod'
import { BashEnvironment, CommandRegistry, type Command, type HostProcess } from '../index'
import { LocalHost } from '@demicodes/host-local'

test('BashEnvironment keeps cwd and env state across shell_exec calls', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-1',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'pwd' })
  expect(first.status).toBe('exited')
  expect(first.stdout.delta).toBe(`${root}\n`)

  const shellId = first.shellId
  await env.exec({ shellId, script: 'cd pkg' })
  const second = await env.exec({ shellId, script: 'pwd' })
  expect(second.stdout.delta).toBe(`${join(root, 'pkg')}\n`)

  await env.exec({ shellId, script: 'export DEMI_TEST_VALUE=kept' })
  const third = await env.exec({ shellId, script: 'printf "$DEMI_TEST_VALUE"' })
  expect(third.stdout.delta).toBe('kept')
})

test('BashEnvironment ephemeral exec never touches the session default shell', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-ephemeral-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const agentSessionId = 'session-ephemeral'
  const first = await env.exec({ agentSessionId, script: 'pwd' })
  expect(first.status).toBe('exited')

  const ephemeral = await env.exec({ agentSessionId, script: 'cd pkg && pwd', ephemeral: true })
  expect(ephemeral.status).toBe('exited')
  expect(ephemeral.shellId).not.toBe(first.shellId)
  if (ephemeral.status === 'exited') expect(ephemeral.stdout.delta).toBe(`${join(root, 'pkg')}\n`)

  // The default shell is untouched: same shell, same cwd.
  const second = await env.exec({ agentSessionId, script: 'pwd' })
  expect(second.shellId).toBe(first.shellId)
  if (second.status === 'exited') expect(second.stdout.delta).toBe(`${root}\n`)

  expect(await env.disposeShell(ephemeral.shellId)).toBe(true)

  await expect(env.exec({ shellId: first.shellId, script: 'pwd', ephemeral: true })).rejects.toThrow(
    'mutually exclusive',
  )
})

test('BashEnvironment ephemeral exec starts in the requested cwd', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-ephemeral-cwd-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const inPkg = await env.exec({ script: 'pwd', ephemeral: true, cwd: join(root, 'pkg') })
  expect(inPkg.status).toBe('exited')
  if (inPkg.status === 'exited') expect(inPkg.stdout.delta).toBe(`${join(root, 'pkg')}\n`)

  await expect(env.exec({ script: 'pwd', cwd: join(root, 'pkg') })).rejects.toThrow('requires "ephemeral"')
  await expect(env.exec({ script: 'pwd', ephemeral: true, cwd: join(root, 'missing') })).rejects.toThrow(
    'not a directory',
  )
})

test('BashEnvironment applies stateful builtins before expanding later commands in the same script', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-inline-state-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-inline-state',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: 'export INLINE_VALUE=now\ncd pkg\nprintf "%s:%s" "$INLINE_VALUE" "$PWD"',
  })

  expect(result.stdout.delta).toBe(`now:${join(root, 'pkg')}`)
})

test('BashEnvironment rejects invalid stateful builtin arguments without corrupting state', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-invalid-builtins-'))
  await mkdir(join(root, 'one'))
  await mkdir(join(root, 'two'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-invalid-builtins',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const cd = await env.exec({ script: 'cd one two' })
  expect(cd.status).toBe('exited')
  if (cd.status !== 'exited') throw new Error('expected exited result')
  expect(cd.exitCode).toBe(2)
  expect(cd.stderr.delta).toContain('cd: too many arguments')
  const pwd = await env.exec({ shellId: cd.shellId, script: 'pwd' })
  expect(pwd.stdout.delta).toBe(`${root}\n`)

  const exported = await env.exec({
    shellId: cd.shellId,
    script: 'export GOOD=ok 1BAD=nope',
  })
  expect(exported.status).toBe('exited')
  if (exported.status !== 'exited') throw new Error('expected exited result')
  expect(exported.exitCode).toBe(1)
  expect(exported.stderr.delta).toContain('not a valid identifier')
  const envState = await env.exec({ shellId: cd.shellId, script: 'env | grep -E "^(GOOD|1BAD)=" || true' })
  expect(envState.stdout.delta).toBe('GOOD=ok\n')
})

test('BashEnvironment unset builtin mutates variables and functions in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-unset-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-unset',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const removed = await env.exec({
    script: 'export REMOVE_ME=gone KEEP_ME=stay; unset REMOVE_ME; printf "%s|%s|%s" "$REMOVE_ME" "$KEEP_ME" "$?"',
  })
  expect(removed.status).toBe('exited')
  if (removed.status !== 'exited') throw new Error('expected exited result')
  expect(removed.exitCode).toBe(0)
  expect(removed.stdout.delta).toBe('|stay|0')

  const invalid = await env.exec({
    shellId: removed.shellId,
    script: 'unset BAD-NAME KEEP_ME; printf "%s|%s" "$KEEP_ME" "$?"',
  })
  expect(invalid.stdout.delta).toBe('|1')
  expect(invalid.stderr.delta).toContain('unset: BAD-NAME: not a valid identifier')

  const functionRemoved = await env.exec({
    shellId: removed.shellId,
    script: 'greet() { printf fn; }; unset -f greet; if greet; then printf kept; else printf missing; fi',
  })
  expect(functionRemoved.stdout.delta).toBe('missing')
  expect(functionRemoved.stderr.delta).toContain('greet')
})

test('BashEnvironment read builtin consumes stdin into the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-read-'))
  await writeFile(join(root, 'lines.txt'), 'one\n two words\nthree\\\\four\n')
  await writeFile(join(root, 'chunk.txt'), 'abcdef\n')
  await writeFile(join(root, 'newline-chunk.txt'), 'ab\ncdef\n')
  await writeFile(join(root, 'space-chunk.txt'), 'abc def\n')
  await writeFile(join(root, 'colon-chunk.txt'), 'ab:cd:ef')
  await writeFile(join(root, 'words-delim.txt'), 'a b:c')
  await writeFile(join(root, 'escaped-delim.txt'), 'a\\:bc')
  await writeFile(join(root, 'escaped-space.txt'), 'a\\ b c\n')
  await writeFile(join(root, 'nul-delim.txt'), 'left\0right\0')
  await writeFile(join(root, 'short-chunk.txt'), 'abc')
  await writeFile(join(root, 'backslash-chunk.txt'), 'a\\bcd\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-read',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const single = await env.exec({ script: 'read REPLY <<< "alpha beta"; printf "%s|%s" "$REPLY" "$?"' })
  expect(single.status).toBe('exited')
  if (single.status !== 'exited') throw new Error('expected exited result')
  expect(single.exitCode).toBe(0)
  expect(single.stdout.delta).toBe('alpha beta|0')

  const replyPreservesWhitespace = await env.exec({
    shellId: single.shellId,
    script: 'read <<< "  alpha beta  "; printf "<%s>:%s" "$REPLY" "$?"',
  })
  expect(replyPreservesWhitespace.stdout.delta).toBe('<  alpha beta  >:0')

  const split = await env.exec({
    shellId: single.shellId,
    script: 'read first rest <<< "alpha beta gamma"; printf "%s|%s|%s" "$first" "$rest" "$?"',
  })
  expect(split.stdout.delta).toBe('alpha|beta gamma|0')

  const emptyIfs = await env.exec({
    shellId: single.shellId,
    script: 'IFS= read first rest <<< "  one two  "; printf "<%s>|<%s>:%s" "$first" "$rest" "$?"',
  })
  expect(emptyIfs.stdout.delta).toBe('<  one two  >|<>:0')

  const nonWhitespaceIfs = await env.exec({
    shellId: single.shellId,
    script: 'IFS=: read a b c d <<< ":one::two:"; printf "<%s>|<%s>|<%s>|<%s>:%s" "$a" "$b" "$c" "$d" "$?"',
  })
  expect(nonWhitespaceIfs.stdout.delta).toBe('<>|<one>|<>|<two>:0')

  const escapedIfs = await env.exec({
    shellId: single.shellId,
    script: 'read first rest < escaped-space.txt; printf "<%s>|<%s>:%s" "$first" "$rest" "$?"',
  })
  expect(escapedIfs.stdout.delta).toBe('<a b>|<c>:0')

  const loop = await env.exec({
    shellId: single.shellId,
    script: 'while IFS= read -r line; do printf "<%s>" "$line"; done < lines.txt; printf "|%s" "$?"',
  })
  expect(loop.stdout.delta).toBe('<one>< two words><three\\\\four>|0')

  const empty = await env.exec({ shellId: single.shellId, script: 'read missing < /dev/null; printf "$?"' })
  expect(empty.stdout.delta).toBe('1')

  const chars = await env.exec({
    shellId: single.shellId,
    script: '{ read -n 3 chunk; printf "<%s>:%s:" "$chunk" "$?"; read rest; printf "<%s>:%s" "$rest" "$?"; } < chunk.txt',
  })
  expect(chars.stdout.delta).toBe('<abc>:0:<def>:0')

  const newline = await env.exec({
    shellId: single.shellId,
    script:
      '{ read -n 5 chunk; printf "<%s>:%s:" "$chunk" "$?"; read rest; printf "<%s>:%s" "$rest" "$?"; } < newline-chunk.txt',
  })
  expect(newline.stdout.delta).toBe('<ab>:0:<cdef>:0')

  const short = await env.exec({
    shellId: single.shellId,
    script: 'read -n 5 chunk < short-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(short.stdout.delta).toBe('<abc>:1')

  const clustered = await env.exec({
    shellId: single.shellId,
    script: 'read -rn3 chunk < backslash-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(clustered.stdout.delta).toBe('<a\\b>:0')

  const exact = await env.exec({
    shellId: single.shellId,
    script: '{ read -N 5 chunk; printf "<%s>:%s:" "$chunk" "$?"; read rest; printf "<%s>:%s" "$rest" "$?"; } < newline-chunk.txt',
  })
  expect(exact.stdout.delta).toBe('<ab\ncd>:0:<ef>:0')

  const exactSplit = await env.exec({
    shellId: single.shellId,
    script: 'read -N 7 first second < space-chunk.txt; printf "<%s>|<%s>:%s" "$first" "$second" "$?"',
  })
  expect(exactSplit.stdout.delta).toBe('<abc def>|<>:0')

  const exactShort = await env.exec({
    shellId: single.shellId,
    script: 'read -N 5 chunk < short-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(exactShort.stdout.delta).toBe('<abc>:1')

  const exactZero = await env.exec({
    shellId: single.shellId,
    script: '{ read -N 0 chunk; printf "<%s>:%s:" "$chunk" "$?"; read rest; printf "<%s>:%s" "$rest" "$?"; } < chunk.txt',
  })
  expect(exactZero.stdout.delta).toBe('<>:0:<abcdef>:0')

  const exactEscaped = await env.exec({
    shellId: single.shellId,
    script: 'read -N 4 chunk < backslash-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(exactEscaped.stdout.delta).toBe('<a\\bc>:0')

  const delimited = await env.exec({
    shellId: single.shellId,
    script: '{ read -d : chunk; printf "<%s>:%s:" "$chunk" "$?"; read rest; printf "<%s>:%s" "$rest" "$?"; } < colon-chunk.txt',
  })
  expect(delimited.stdout.delta).toBe('<ab>:0:<cd:ef>:1')

  const delimitedSplit = await env.exec({
    shellId: single.shellId,
    script: 'read -d : first second < words-delim.txt; printf "<%s>|<%s>:%s" "$first" "$second" "$?"',
  })
  expect(delimitedSplit.stdout.delta).toBe('<a>|<b>:0')

  const escapedDelimiter = await env.exec({
    shellId: single.shellId,
    script: 'read -d : chunk < escaped-delim.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(escapedDelimiter.stdout.delta).toBe('<a:bc>:1')

  const rawDelimiter = await env.exec({
    shellId: single.shellId,
    script: 'read -rd : chunk < escaped-delim.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(rawDelimiter.stdout.delta).toBe('<a\\>:0')

  const nulDelimiter = await env.exec({
    shellId: single.shellId,
    script: 'read -d "" chunk < nul-delim.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(nulDelimiter.stdout.delta).toBe('<left>:0')

  const charsWithDelimiter = await env.exec({
    shellId: single.shellId,
    script: 'read -d : -n 5 chunk < colon-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(charsWithDelimiter.stdout.delta).toBe('<ab>:0')

  const exactIgnoresDelimiter = await env.exec({
    shellId: single.shellId,
    script: 'read -d : -N 5 chunk < colon-chunk.txt; printf "<%s>:%s" "$chunk" "$?"',
  })
  expect(exactIgnoresDelimiter.stdout.delta).toBe('<ab:cd>:0')
})

test('BashEnvironment honors shell list operators without falling back to a system shell', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-list-operators',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const recovered = await env.exec({ script: 'false && printf no || printf yes' })
  expect(recovered.status).toBe('exited')
  if (recovered.status !== 'exited') throw new Error('expected exited result')
  expect(recovered.exitCode).toBe(0)
  expect(recovered.stdout.delta).toBe('yes')

  const success = await env.exec({ shellId: recovered.shellId, script: 'true && printf ok' })
  expect(success.stdout.delta).toBe('ok')

  const continued = await env.exec({ shellId: recovered.shellId, script: 'false; printf after' })
  expect(continued.status).toBe('exited')
  if (continued.status !== 'exited') throw new Error('expected exited result')
  expect(continued.exitCode).toBe(0)
  expect(continued.stdout.delta).toBe('after')
})

test('BashEnvironment supports prefix assignments and assignment-only commands', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-assignments',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const temporary = await env.exec({ script: 'NAME=Alice sh -c \'printf "$NAME"\'' })
  expect(temporary.stdout.delta).toBe('Alice')
  const notPersisted = await env.exec({ shellId: temporary.shellId, script: 'printf "$NAME"' })
  expect(notPersisted.stdout.delta).toBe('')

  await env.exec({ shellId: temporary.shellId, script: 'PERSIST=kept' })
  const persisted = await env.exec({ shellId: temporary.shellId, script: 'printf "$PERSIST"' })
  expect(persisted.stdout.delta).toBe('kept')

  await env.exec({ shellId: temporary.shellId, script: 'PERSIST+=-more' })
  const appended = await env.exec({ shellId: temporary.shellId, script: 'printf "$PERSIST"' })
  expect(appended.stdout.delta).toBe('kept-more')

  const piped = await env.exec({ shellId: temporary.shellId, script: 'NAME=Bob sh -c \'printf "$NAME"\' | tr a-z A-Z' })
  expect(piped.stdout.delta).toBe('BOB')
})

test('BashEnvironment keeps last exit status in $? across commands and exec calls', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-last-status',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const recovered = await env.exec({ script: 'false; printf "%s\\n" "$?"; true; printf "%s\\n" "$?"' })
  expect(recovered.stdout.delta).toBe('1\n0\n')

  await env.exec({ shellId: recovered.shellId, script: 'false' })
  const crossExec = await env.exec({ shellId: recovered.shellId, script: 'printf "$?"' })
  expect(crossExec.stdout.delta).toBe('1')

  const list = await env.exec({ shellId: recovered.shellId, script: 'false || printf "$?"' })
  expect(list.stdout.delta).toBe('1')

  const pipeline = await env.exec({ shellId: recovered.shellId, script: 'false | true; printf "$?"' })
  expect(pipeline.stdout.delta).toBe('0')
})

test('BashEnvironment exit builtin marks the shell session exited', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const exited = await env.exec({ script: 'printf before; exit 7; printf after' })

  expect(exited.status).toBe('exited')
  if (exited.status !== 'exited') throw new Error('expected exited result')
  expect(exited.exitCode).toBe(7)
  expect(exited.stdout.delta).toBe('before')
  expect(env.getShell(exited.shellId)?.exited).toBe(true)
  await expect(env.exec({ shellId: exited.shellId, script: 'printf unreachable' })).rejects.toThrow(
    'has exited',
  )
})

test('BashEnvironment exit builtin follows shell exit status rules', async () => {
  let nextSession = 0
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => `shell-exit-rules-${++nextSession}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const inherited = await env.exec({ script: 'false; exit' })
  expect(inherited.status).toBe('exited')
  if (inherited.status !== 'exited') throw new Error('expected exited result')
  expect(inherited.exitCode).toBe(1)

  const invalid = await env.exec({ script: 'exit bad' })
  expect(invalid.status).toBe('exited')
  if (invalid.status !== 'exited') throw new Error('expected exited result')
  expect(invalid.exitCode).toBe(2)
  expect(invalid.stderr.delta).toContain('numeric argument required')

  const wrapped = await env.exec({ script: 'exit 300' })
  expect(wrapped.status).toBe('exited')
  if (wrapped.status !== 'exited') throw new Error('expected exited result')
  expect(wrapped.exitCode).toBe(44)

  const tooMany = await env.exec({ script: 'exit 1 2; printf unreachable' })
  expect(tooMany.status).toBe('exited')
  if (tooMany.status !== 'exited') throw new Error('expected exited result')
  expect(tooMany.exitCode).toBe(1)
  expect(tooMany.stderr.delta).toContain('too many arguments')
  expect(tooMany.stdout.delta).toBe('')
})

test('BashEnvironment supports common parameter expansion operations', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-parameter-ops',
    initialEnv: { PATH: process.env.PATH ?? '', EMPTY: '', SET_VALUE: 'value' },
  })

  const defaults = await env.exec({
    script: `printf "%s:%s:%s:%s" "\${MISSING:-guest}" "\${EMPTY:-guest}" "\${EMPTY-plain}" "\${SET_VALUE:-guest}"`,
  })
  expect(defaults.stdout.delta).toBe('guest:guest::value')

  const assigned = await env.exec({ shellId: defaults.shellId, script: `printf "\${ASSIGN_ME:=created}"` })
  expect(assigned.stdout.delta).toBe('created')
  const persisted = await env.exec({ shellId: defaults.shellId, script: 'printf "$ASSIGN_ME"' })
  expect(persisted.stdout.delta).toBe('created')

  const alternative = await env.exec({
    shellId: defaults.shellId,
    script: `printf "%s:%s:%s" "\${SET_VALUE:+yes}" "\${EMPTY:+yes}" "\${MISSING+yes}"`,
  })
  expect(alternative.stdout.delta).toBe('yes::')

  const patternOps = await env.exec({
    shellId: defaults.shellId,
    script: `PATH_VALUE=src/app.test.ts; printf "%s:%s:%s:%s:%s:%s:%s" "\${#PATH_VALUE}" "\${PATH_VALUE#*/}" "\${PATH_VALUE##*.}" "\${PATH_VALUE##*[./]}" "\${PATH_VALUE%.*}" "\${PATH_VALUE%%/*}" "\${PATH_VALUE#missing}"`,
  })
  expect(patternOps.stdout.delta).toBe('15:app.test.ts:ts:ts:src/app.test:src:src/app.test.ts')

  const replacements = await env.exec({
    shellId: defaults.shellId,
    script: `REPL=foo-bar-foo; printf "%s:%s:%s:%s:%s" "\${REPL/foo/baz}" "\${REPL//foo/baz}" "\${REPL/#foo/baz}" "\${REPL/%foo/baz}" "\${REPL/-bar}"`,
  })
  expect(replacements.stdout.delta).toBe('baz-bar-foo:baz-bar-baz:baz-bar-foo:foo-bar-baz:foo-foo')

  const substring = await env.exec({
    shellId: defaults.shellId,
    script: `SUB=abcdef; i=1; printf "%s:%s:%s:%s:%s:%s" "\${SUB:1}" "\${SUB:1:3}" "\${SUB: -2}" "\${SUB: -4:2}" "\${SUB:1:-1}" "\${SUB:i++:2}:$i"`,
  })
  expect(substring.stdout.delta).toBe('bcdef:bcd:ef:cd:bcde:bc:2')

  const caseAndIndirect = await env.exec({
    shellId: defaults.shellId,
    script: `CASE_VALUE=heLLo; PATTERN_VALUE=hello; TARGET_NAME=SET_VALUE; printf "%s:%s:%s:%s:%s:%s" "\${CASE_VALUE^}" "\${CASE_VALUE^^}" "\${CASE_VALUE,}" "\${CASE_VALUE,,}" "\${PATTERN_VALUE^^[lo]}" "\${!TARGET_NAME}"`,
  })
  expect(caseAndIndirect.stdout.delta).toBe('HeLLo:HELLO:heLLo:hello:heLLO:value')

  await expect(env.exec({ shellId: defaults.shellId, script: `printf "\${MISSING:?required}"` })).rejects.toThrow(
    'MISSING: required',
  )
  await expect(env.exec({ shellId: defaults.shellId, script: `SUB=abc; printf "\${SUB:2:-5}"` })).rejects.toThrow(
    'substring expression < 0',
  )
})

test('BashEnvironment supports arithmetic expansion in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-arithmetic-expansion',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script:
      'i=1; printf "%s|" $((1 + 2 * 3)); printf "%s|%s|" $((i++)) "$i"; printf "%s|%s|" $((++i)) $((i += 3)); printf "%s|%s|%s" $(((1 + 2) * 3)) $((i > 4 ? 8 : 9)) "$i"',
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.stdout.delta).toBe('7|1|2|3|6|9|8|6')

  const loop = await env.exec({
    shellId: result.shellId,
    script: 'i=0; while test "$i" -lt 3; do printf "$i"; i=$((i + 1)); done; printf "|$i|$?"',
  })
  expect(loop.stdout.delta).toBe('012|3|0')
})

test('BashEnvironment supports arithmetic commands in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-arithmetic-command',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const nonZero = await env.exec({ script: 'i=0; (( i += 2 )); printf "%s|%s" "$i" "$?"' })
  expect(nonZero.status).toBe('exited')
  if (nonZero.status !== 'exited') throw new Error('expected exited result')
  expect(nonZero.stdout.delta).toBe('2|0')

  const zero = await env.exec({ shellId: nonZero.shellId, script: '(( i -= 2 )); printf "%s|%s" "$i" "$?"' })
  expect(zero.stdout.delta).toBe('0|1')

  const loop = await env.exec({
    shellId: nonZero.shellId,
    script: 'i=0; while (( i < 3 )); do printf "$i"; (( i++ )); done; printf "|$i|$?"',
  })
  expect(loop.stdout.delta).toBe('012|3|0')

  const divided = await env.exec({ shellId: nonZero.shellId, script: 'i=8; (( i /= 2 )); printf "%s|%s" "$i" "$?"' })
  expect(divided.stdout.delta).toBe('4|0')

  const negated = await env.exec({ shellId: nonZero.shellId, script: '! (( 0 )); printf "$?"' })
  expect(negated.stdout.delta).toBe('0')
})

test('BashEnvironment expands command substitutions in the current shell context', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-command-sub-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-command-substitution',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const quoted = await env.exec({
    script: 'export NAME=Alice\ncd pkg\nprintf "%s" "$(printf "$NAME:$PWD")"',
  })
  expect(quoted.stdout.delta).toBe(`Alice:${join(root, 'pkg')}`)

  const subshell = await env.exec({
    shellId: quoted.shellId,
    script: 'printf "%s" "$(cd ..; export INNER=changed; printf "$PWD:$INNER")"',
  })
  expect(subshell.stdout.delta).toBe(`${root}:changed`)

  const state = await env.exec({ shellId: quoted.shellId, script: 'printf "%s:%s" "$PWD" "$INNER"' })
  expect(state.stdout.delta).toBe(`${join(root, 'pkg')}:`)

  const quotedNewlines = await env.exec({
    shellId: quoted.shellId,
    script: 'printf "<%s>" "$(printf "a\\nb\\n\\n")"',
  })
  expect(quotedNewlines.stdout.delta).toBe('<a\nb>')

  const unquotedNewlines = await env.exec({
    shellId: quoted.shellId,
    script: 'echo $(printf "a\\nb\\n")',
  })
  expect(unquotedNewlines.stdout.delta).toBe('a b\n')

  const legacy = await env.exec({ shellId: quoted.shellId, script: 'printf "%s" `printf legacy`' })
  expect(legacy.stdout.delta).toBe('legacy')
})

test('BashEnvironment uses command substitution status for assignment-only commands', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-command-sub-assignment',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const failed = await env.exec({ script: 'ASSIGNED=$(false)' })
  expect(failed.status).toBe('exited')
  if (failed.status !== 'exited') throw new Error('expected exited result')
  expect(failed.exitCode).toBe(1)

  const lastStatus = await env.exec({ shellId: failed.shellId, script: 'printf "%s:%s" "$ASSIGNED" "$?"' })
  expect(lastStatus.stdout.delta).toBe(':1')

  const succeeded = await env.exec({ shellId: failed.shellId, script: 'ASSIGNED=$(printf ok)' })
  expect(succeeded.status).toBe('exited')
  if (succeeded.status !== 'exited') throw new Error('expected exited result')
  expect(succeeded.exitCode).toBe(0)

  const value = await env.exec({ shellId: failed.shellId, script: 'printf "%s:%s" "$ASSIGNED" "$?"' })
  expect(value.stdout.delta).toBe('ok:0')

  const pipeline = await env.exec({
    shellId: failed.shellId,
    script: 'printf "$(false)" | PIPE_ASSIGNED=ok; printf "%s:%s" "$PIPE_ASSIGNED" "$?"',
  })
  expect(pipeline.stdout.delta).toBe('ok:0')
})

test('BashEnvironment keeps pushd/popd directory stack across shell_exec calls', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-dirstack-'))
  await mkdir(join(root, 'one'))
  await mkdir(join(root, 'two'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-dirstack',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'pushd one' })
  expect(first.stdout.delta).toBe(`${join(root, 'one')} ${root}\n`)

  const second = await env.exec({ shellId: first.shellId, script: 'pushd ../two' })
  expect(second.stdout.delta).toBe(`${join(root, 'two')} ${join(root, 'one')} ${root}\n`)

  const dirs = await env.exec({ shellId: first.shellId, script: 'dirs' })
  expect(dirs.stdout.delta).toBe(`${join(root, 'two')} ${join(root, 'one')} ${root}\n`)

  const swapped = await env.exec({ shellId: first.shellId, script: 'pushd' })
  expect(swapped.stdout.delta).toBe(`${join(root, 'one')} ${join(root, 'two')} ${root}\n`)

  const pop = await env.exec({ shellId: first.shellId, script: 'popd' })
  expect(pop.stdout.delta).toBe(`${join(root, 'two')} ${root}\n`)
  const pwd = await env.exec({ shellId: first.shellId, script: 'pwd' })
  expect(pwd.stdout.delta).toBe(`${join(root, 'two')}\n`)

  await env.exec({ shellId: first.shellId, script: 'popd' })
  const empty = await env.exec({ shellId: first.shellId, script: 'popd' })
  expect(empty.status).toBe('exited')
  if (empty.status !== 'exited') throw new Error('expected exited result')
  expect(empty.exitCode).toBe(1)
  expect(empty.stderr.delta).toContain('directory stack empty')
})

test('BashEnvironment keeps shell functions across shell_exec calls', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-functions-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-functions',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const defined = await env.exec({
    script: 'set -- outer1 outer2\nsay() { printf "%s:%s:%s\\n" "$#" "$1" "$2"; }\ngo_pkg() { cd pkg\nexport FN_VALUE=kept\n}',
  })

  const called = await env.exec({ shellId: defined.shellId, script: 'say inner1 inner2' })
  expect(called.stdout.delta).toBe('2:inner1:inner2\n')

  const restored = await env.exec({ shellId: defined.shellId, script: 'printf "%s:%s:%s\\n" "$#" "$1" "$2"' })
  expect(restored.stdout.delta).toBe('2:outer1:outer2\n')

  await env.exec({ shellId: defined.shellId, script: 'go_pkg' })
  const state = await env.exec({ shellId: defined.shellId, script: 'printf "%s:%s" "$PWD" "$FN_VALUE"' })
  expect(state.stdout.delta).toBe(`${join(root, 'pkg')}:kept`)
})

test('BashEnvironment supports local variables and return inside functions and sourced scripts', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-local-return-'))
  await writeFile(join(root, 'returning.sh'), 'printf sourced-before; return 9; printf sourced-after\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-local-return',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const scoped = await env.exec({
    script: [
      'VALUE=outer',
      'fn() { local VALUE=inner created=temp empty; printf "%s:%s:%s|" "$VALUE" "$created" "$empty"; }',
      'fn',
      'printf "%s:%s:%s" "$VALUE" "$created" "$?"',
    ].join('\n'),
  })
  expect(scoped.status).toBe('exited')
  if (scoped.status !== 'exited') throw new Error('expected exited result')
  expect(scoped.stdout.delta).toBe('inner:temp:|outer::0')

  const returned = await env.exec({
    shellId: scoped.shellId,
    script: 'early() { local VALUE=inside; printf before; return 7; printf after; }\nearly\nprintf "|%s:%s" "$VALUE" "$?"',
  })
  expect(returned.stdout.delta).toBe('before|outer:7')

  const sourced = await env.exec({ shellId: scoped.shellId, script: 'source returning.sh; printf "|%s" "$?"' })
  expect(sourced.stdout.delta).toBe('sourced-before|9')

  const outsideLocal = await env.exec({ shellId: scoped.shellId, script: 'local OUTSIDE=bad' })
  expect(outsideLocal.status).toBe('exited')
  if (outsideLocal.status !== 'exited') throw new Error('expected exited result')
  expect(outsideLocal.exitCode).toBe(1)
  expect(outsideLocal.stderr.delta).toContain('local: can only be used in a function')

  const outsideReturn = await env.exec({ shellId: scoped.shellId, script: 'return 3' })
  expect(outsideReturn.status).toBe('exited')
  if (outsideReturn.status !== 'exited') throw new Error('expected exited result')
  expect(outsideReturn.exitCode).toBe(1)
  expect(outsideReturn.stderr.delta).toContain("return: can only 'return'")
})

test('BashEnvironment applies function definition redirections at call time', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-function-redir-'))
  await writeFile(join(root, 'input.txt'), 'from-input')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-function-redir',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const defined = await env.exec({
    script: 'TARGET=first; writer() { printf "$1"; } > "$TARGET.txt"; TARGET=second; writer call-one; cat second.txt',
  })
  expect(defined.status).toBe('exited')
  if (defined.status !== 'exited') throw new Error('expected exited result')
  expect(defined.stdout.delta).toBe('call-one')

  const invocationRedirection = await env.exec({
    shellId: defined.shellId,
    script: 'writer call-two > call.txt; printf "second=%s call=%s" "$(cat second.txt)" "$(cat call.txt 2>/dev/null)"',
  })
  expect(invocationRedirection.stdout.delta).toBe('second=call-two call=')

  const input = await env.exec({
    shellId: defined.shellId,
    script: 'reader() { cat; } < "$1"; reader input.txt',
  })
  expect(input.stdout.delta).toBe('from-input')

  const combined = await env.exec({
    shellId: defined.shellId,
    script: 'talk() { printf out; printf err >&2; } > both.txt 2>&1; talk; cat both.txt',
  })
  expect(combined.stdout.delta).toBe('outerr')
})

test('BashEnvironment keeps background jobs across shell_exec calls', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-jobs-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-jobs',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const started = await env.exec({ script: 'sh -c "sleep 0.05; printf bg-done" &' })
  expect(started.status).toBe('exited')
  expect(started.stdout.delta).toContain('[1] sh -c')

  const listed = await env.exec({ shellId: started.shellId, script: 'jobs' })
  expect(listed.stdout.delta).toContain('[1] Running')

  const waited = await env.exec({ shellId: started.shellId, script: 'wait %1' })
  expect(waited.status).toBe('exited')
  if (waited.status !== 'exited') throw new Error('expected exited result')
  expect(waited.exitCode).toBe(0)
  expect(waited.stdout.delta).toBe('bg-done')

  const empty = await env.exec({ shellId: started.shellId, script: 'jobs' })
  expect(empty.stdout.delta).toBe('')
})

test('BashEnvironment reports background command spawn failures through wait', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-bg-spawn-failure-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-bg-spawn-failure',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const started = await env.exec({ script: 'demi-background-command-that-does-not-exist &' })
  expect(started.status).toBe('exited')
  expect(started.stdout.delta).toContain('[1] demi-background-command-that-does-not-exist')

  const waited = await env.exec({ shellId: started.shellId, script: 'wait %1' })
  expect(waited.status).toBe('exited')
  if (waited.status !== 'exited') throw new Error('expected exited result')
  expect(waited.exitCode).toBe(127)
  expect(waited.stderr.delta).toContain('demi-background-command-that-does-not-exist')
})

test('BashEnvironment source builtin mutates the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-source-'))
  await mkdir(join(root, 'pkg'))
  await writeFile(join(root, 'env.sh'), 'export SOURCED_VALUE=from-source\ncd pkg\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-source',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const sourced = await env.exec({ script: 'source env.sh' })
  expect(sourced.status).toBe('exited')

  const state = await env.exec({ shellId: sourced.shellId, script: 'sh -c \'printf "%s:%s" "$SOURCED_VALUE" "$PWD"\'' })
  expect(state.stdout.delta).toBe(`from-source:${join(root, 'pkg')}`)
})

test('BashEnvironment source builtin resolves slashless files from PATH', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-source-path-'))
  await mkdir(join(root, 'scripts'))
  await writeFile(join(root, 'scripts', 'from-path.sh'), 'export SOURCED_FROM_PATH=ok\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-source-path',
    initialEnv: { PATH: `${join(root, 'scripts')}:${process.env.PATH ?? ''}` },
  })

  const sourced = await env.exec({ script: 'source from-path.sh; printf "$SOURCED_FROM_PATH"' })
  expect(sourced.status).toBe('exited')
  if (sourced.status !== 'exited') throw new Error('expected exited result')
  expect(sourced.exitCode).toBe(0)
  expect(sourced.stdout.delta).toBe('ok')
})

test('BashEnvironment source builtin inherits input redirections', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-source-input-'))
  await writeFile(join(root, 'reader.sh'), 'read SOURCED_INPUT\nexport SOURCED_INPUT\n')
  await writeFile(join(root, 'input.txt'), 'from-input\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-source-input',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const sourced = await env.exec({ script: 'source reader.sh < input.txt; printf "$SOURCED_INPUT"' })
  expect(sourced.status).toBe('exited')
  if (sourced.status !== 'exited') throw new Error('expected exited result')
  expect(sourced.exitCode).toBe(0)
  expect(sourced.stdout.delta).toBe('from-input')
})

test('BashEnvironment set -- and shift mutate session positional parameters', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-shift',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const set = await env.exec({ script: 'set -- alpha "bravo charlie" delta' })
  const before = await env.exec({ shellId: set.shellId, script: 'printf "%s:%s:%s" "$#" "$1" "$2"' })
  expect(before.stdout.delta).toBe('3:alpha:bravo charlie')

  const shifted = await env.exec({ shellId: set.shellId, script: 'shift 2' })
  expect(shifted.status).toBe('exited')
  if (shifted.status !== 'exited') throw new Error('expected exited result')
  expect(shifted.exitCode).toBe(0)

  const after = await env.exec({ shellId: set.shellId, script: 'printf "%s:%s:%s" "$#" "$1" "$2"' })
  expect(after.stdout.delta).toBe('1:delta:')

  const tooFar = await env.exec({ shellId: set.shellId, script: 'shift 2' })
  expect(tooFar.status).toBe('exited')
  if (tooFar.status !== 'exited') throw new Error('expected exited result')
  expect(tooFar.exitCode).toBe(1)
  const unchanged = await env.exec({ shellId: set.shellId, script: 'printf "%s:%s" "$#" "$1"' })
  expect(unchanged.stdout.delta).toBe('1:delta')

  const tooMany = await env.exec({ shellId: set.shellId, script: 'shift 1 2' })
  expect(tooMany.status).toBe('exited')
  if (tooMany.status !== 'exited') throw new Error('expected exited result')
  expect(tooMany.exitCode).toBe(1)
  expect(tooMany.stderr.delta).toContain('too many arguments')
  const afterTooMany = await env.exec({ shellId: set.shellId, script: 'printf "%s:%s" "$#" "$1"' })
  expect(afterTooMany.stdout.delta).toBe('1:delta')

  const bad = await env.exec({ shellId: set.shellId, script: 'shift bad' })
  expect(bad.status).toBe('exited')
  if (bad.status !== 'exited') throw new Error('expected exited result')
  expect(bad.exitCode).toBe(1)
  expect(bad.stderr.delta).toContain('numeric argument required')
  const afterBad = await env.exec({ shellId: set.shellId, script: 'printf "%s:%s" "$#" "$1"' })
  expect(afterBad.stdout.delta).toBe('1:delta')
})

test('BashEnvironment set builtin manages errexit in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-set-errexit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const stopped = await env.exec({ script: 'set -e; false; printf after' })
  expect(stopped.status).toBe('exited')
  if (stopped.status !== 'exited') throw new Error('expected exited result')
  expect(stopped.exitCode).toBe(1)
  expect(stopped.stdout.delta).toBe('')
  expect(stopped.audit.map((event) => ('name' in event ? event.name : ''))).not.toContain('set')

  const alive = await env.exec({ shellId: stopped.shellId, script: 'printf alive' })
  expect(alive.stdout.delta).toBe('alive')

  const disabled = await env.exec({ shellId: stopped.shellId, script: 'set +e; false; printf after' })
  expect(disabled.status).toBe('exited')
  if (disabled.status !== 'exited') throw new Error('expected exited result')
  expect(disabled.exitCode).toBe(0)
  expect(disabled.stdout.delta).toBe('after')

  const controlled = await env.exec({
    shellId: stopped.shellId,
    script: 'set -e; false || printf recovered; if false; then printf bad; else printf else; fi; printf done',
  })
  expect(controlled.status).toBe('exited')
  if (controlled.status !== 'exited') throw new Error('expected exited result')
  expect(controlled.exitCode).toBe(0)
  expect(controlled.stdout.delta).toBe('recoveredelsedone')

  const optionForm = await env.exec({ shellId: stopped.shellId, script: 'set -o errexit; true && false; printf after' })
  expect(optionForm.status).toBe('exited')
  if (optionForm.status !== 'exited') throw new Error('expected exited result')
  expect(optionForm.exitCode).toBe(1)
  expect(optionForm.stdout.delta).toBe('')

  const plusOption = await env.exec({ shellId: stopped.shellId, script: 'set +o errexit; false; printf after' })
  expect(plusOption.stdout.delta).toBe('after')

  const unsupported = await env.exec({ shellId: stopped.shellId, script: 'set -z' })
  expect(unsupported.status).toBe('exited')
  if (unsupported.status !== 'exited') throw new Error('expected exited result')
  expect(unsupported.exitCode).toBe(2)
  expect(unsupported.stderr.delta).toContain('invalid option')
  expect(unsupported.audit).toEqual([])
})

test('BashEnvironment set builtin manages noglob in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-set-noglob-'))
  await writeFile(join(root, 'alpha.ts'), '')
  await writeFile(join(root, 'beta.ts'), '')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-set-noglob',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const disabled = await env.exec({ script: 'set -f; printf "%s|" *.ts' })
  expect(disabled.status).toBe('exited')
  if (disabled.status !== 'exited') throw new Error('expected exited result')
  expect(disabled.exitCode).toBe(0)
  expect(disabled.stdout.delta).toBe('*.ts|')
  expect(disabled.audit.map((event) => ('name' in event ? event.name : ''))).not.toContain('set')

  const restored = await env.exec({ shellId: disabled.shellId, script: 'set +f; printf "%s|" *.ts' })
  expect(restored.stdout.delta).toBe('alpha.ts|beta.ts|')

  const optionForm = await env.exec({
    shellId: disabled.shellId,
    script: 'set -o noglob; for file in *.ts; do printf "<%s>" "$file"; done',
  })
  expect(optionForm.stdout.delta).toBe('<*.ts>')

  const plusOption = await env.exec({
    shellId: disabled.shellId,
    script: 'set +o noglob; for file in *.ts; do printf "<%s>" "$file"; done',
  })
  expect(plusOption.stdout.delta).toBe('<alpha.ts><beta.ts>')
})

test('BashEnvironment eval builtin executes in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-eval-'))
  await mkdir(join(root, 'pkg'))
  await writeFile(join(root, 'input.txt'), 'from-input\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-eval',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const stateful = await env.exec({
    script: 'eval "cd pkg; export EVAL_VALUE=kept; greet() { printf hi; }"; greet; printf "|%s|%s" "$EVAL_VALUE" "$PWD"',
  })
  expect(stateful.status).toBe('exited')
  if (stateful.status !== 'exited') throw new Error('expected exited result')
  expect(stateful.stdout.delta).toBe(`hi|kept|${join(root, 'pkg')}`)
  expect(stateful.audit.map((event) => ('name' in event ? event.name : ''))).not.toContain('eval')

  const args = await env.exec({ shellId: stateful.shellId, script: 'eval printf "%s:%s" "$EVAL_VALUE" "$PWD"' })
  expect(args.stdout.delta).toBe(`kept:${join(root, 'pkg')}`)

  const inheritedInput = await env.exec({ shellId: stateful.shellId, script: 'eval "read EVAL_INPUT" < ../input.txt; printf "$EVAL_INPUT"' })
  expect(inheritedInput.stdout.delta).toBe('from-input')

  const commandBuiltin = await env.exec({ shellId: stateful.shellId, script: 'command eval "printf command-eval"' })
  expect(commandBuiltin.stdout.delta).toBe('command-eval')

  const empty = await env.exec({ shellId: stateful.shellId, script: 'eval -- ""' })
  expect(empty.status).toBe('exited')
  if (empty.status !== 'exited') throw new Error('expected exited result')
  expect(empty.exitCode).toBe(0)

  const invalidOption = await env.exec({ shellId: stateful.shellId, script: 'eval -z "printf bad"' })
  expect(invalidOption.status).toBe('exited')
  if (invalidOption.status !== 'exited') throw new Error('expected exited result')
  expect(invalidOption.exitCode).toBe(2)
  expect(invalidOption.stderr.delta).toContain('invalid option')
  expect(invalidOption.audit).toEqual([])

  const syntax = await env.exec({ shellId: stateful.shellId, script: 'eval "time printf bad"' })
  expect(syntax.status).toBe('exited')
  if (syntax.status !== 'exited') throw new Error('expected exited result')
  expect(syntax.exitCode).toBe(2)
  expect(syntax.stderr.delta).toContain('Unsupported shell syntax')
})

test('BashEnvironment source args temporarily replace positional parameters', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-source-args-'))
  await writeFile(
    join(root, 'positional.sh'),
    'printf "inside-before:%s:%s:%s\\n" "$#" "$1" "$2"\nshift\nprintf "inside-after:%s:%s:%s\\n" "$#" "$1" "$2"\n',
  )
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-source-args',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const set = await env.exec({ script: 'set -- outer1 outer2' })
  const sourced = await env.exec({ shellId: set.shellId, script: 'source positional.sh inner1 inner2' })
  expect(sourced.stdout.delta).toBe('inside-before:2:inner1:inner2\ninside-after:1:inner2:\n')

  const restored = await env.exec({ shellId: set.shellId, script: 'printf "after:%s:%s:%s" "$#" "$1" "$2"' })
  expect(restored.stdout.delta).toBe('after:2:outer1:outer2')
})

test('BashEnvironment dispatches registered commands before system commands and records audit', async () => {
  const registry = new CommandRegistry()
  registry.register(echoCommand())
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: registry,
    shellIdFactory: () => 'shell-registered',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: 'echo-cmd run --value hello' })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.stdout.delta).toBe('registered:hello\n')
  expect(result.audit).toEqual([
    {
      kind: 'registered-command',
      name: 'echo-cmd',
      args: ['run', '--value', 'hello'],
      exitCode: 0,
    },
  ])
  expect(result.commandMetadata).toEqual([
    {
      kind: 'registered-command',
      name: 'echo-cmd',
      args: ['run', '--value', 'hello'],
      metadata: { echoed: 'hello' },
    },
  ])
})

test('BashEnvironment supports command builtin lookup and system execution', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-command-builtin-'))
  await mkdir(join(root, 'next'))
  const registry = new CommandRegistry()
  registry.register(echoCommand())
  const env = new BashEnvironment({
    host: new LocalHost(root),
    commands: registry,
    shellIdFactory: () => 'shell-command-builtin',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const lookup = await env.exec({
    script: 'helper() { true; }\ncommand -v cd helper echo-cmd sh demi-missing-command',
  })
  expect(lookup.status).toBe('exited')
  if (lookup.status !== 'exited') throw new Error('expected exited result')
  expect(lookup.exitCode).toBe(1)
  expect(lookup.stderr.delta).toBe('')
  const lookupLines = lookup.stdout.delta.split('\n')
  expect(lookupLines[0]).toBe('cd')
  expect(lookupLines[1]).toBe('helper')
  expect(lookupLines[2]).toBe('echo-cmd')
  expect(lookupLines[3]).toMatch(/\/sh$/)
  expect(lookup.audit).toEqual([])

  const run = await env.exec({ shellId: lookup.shellId, script: 'command sh -c "printf run"' })
  expect(run.status).toBe('exited')
  if (run.status !== 'exited') throw new Error('expected exited result')
  expect(run.exitCode).toBe(0)
  expect(run.stdout.delta).toBe('run')
  expect(run.audit).toEqual([
    {
      kind: 'system-command',
      name: 'sh',
      args: ['-c', 'printf run'],
      cwd: root,
      exitCode: 0,
    },
  ])

  const registeredPriority = await env.exec({
    shellId: lookup.shellId,
    script: 'echo-cmd() { printf function-shadow; }\necho-cmd run --value priority',
  })
  expect(registeredPriority.status).toBe('exited')
  if (registeredPriority.status !== 'exited') throw new Error('expected exited result')
  expect(registeredPriority.exitCode).toBe(0)
  expect(registeredPriority.stdout.delta).toBe('registered:priority\n')
  expect(registeredPriority.audit).toEqual([
    {
      kind: 'registered-command',
      name: 'echo-cmd',
      args: ['run', '--value', 'priority'],
      exitCode: 0,
    },
  ])

  const commandRegistered = await env.exec({
    shellId: lookup.shellId,
    script: 'echo-cmd() { printf function-shadow; }\ncommand echo-cmd run --value direct',
  })
  expect(commandRegistered.status).toBe('exited')
  if (commandRegistered.status !== 'exited') throw new Error('expected exited result')
  expect(commandRegistered.exitCode).toBe(0)
  expect(commandRegistered.stdout.delta).toBe('registered:direct\n')
  expect(commandRegistered.audit).toEqual([
    {
      kind: 'registered-command',
      name: 'echo-cmd',
      args: ['run', '--value', 'direct'],
      exitCode: 0,
    },
  ])

  const commandCd = await env.exec({
    shellId: lookup.shellId,
    script: 'command cd next; pwd',
  })
  expect(commandCd.status).toBe('exited')
  if (commandCd.status !== 'exited') throw new Error('expected exited result')
  expect(commandCd.exitCode).toBe(0)
  expect(commandCd.stdout.delta).toBe(`${join(root, 'next')}\n`)

  const skippedFunction = await env.exec({
    shellId: lookup.shellId,
    script: 'function_only() { printf function; }\ncommand function_only',
  })
  expect(skippedFunction.status).toBe('exited')
  if (skippedFunction.status !== 'exited') throw new Error('expected exited result')
  expect(skippedFunction.exitCode).toBe(127)
  expect(skippedFunction.stdout.delta).toBe('')
  expect(skippedFunction.stderr.delta).toContain('function_only')
  expect(skippedFunction.audit).toEqual([
    {
      kind: 'system-command',
      name: 'function_only',
      args: [],
      cwd: join(root, 'next'),
      exitCode: 127,
    },
  ])
})

test('BashEnvironment type builtin reports session command resolution', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-type-builtin-'))
  const registry = new CommandRegistry()
  registry.register(echoCommand())
  const env = new BashEnvironment({
    host: new LocalHost(root),
    commands: registry,
    shellIdFactory: () => 'shell-type-builtin',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const descriptions = await env.exec({
    script: 'helper() { true; }\necho-cmd() { printf shadow; }\ntype cd echo-cmd helper sh demi-missing-command',
  })
  expect(descriptions.status).toBe('exited')
  if (descriptions.status !== 'exited') throw new Error('expected exited result')
  expect(descriptions.exitCode).toBe(1)
  expect(descriptions.stderr.delta).toContain('type: demi-missing-command: not found')
  const descriptionLines = descriptions.stdout.delta.trimEnd().split('\n')
  expect(descriptionLines[0]).toBe('cd is a shell builtin')
  expect(descriptionLines[1]).toBe('echo-cmd is a registered command')
  expect(descriptionLines[2]).toBe('helper is a shell function')
  expect(descriptionLines[3]).toMatch(/^sh is .*\/sh$/)
  expect(descriptions.audit).toEqual([])

  const kinds = await env.exec({ shellId: descriptions.shellId, script: 'command type -t cd echo-cmd helper sh' })
  expect(kinds.status).toBe('exited')
  if (kinds.status !== 'exited') throw new Error('expected exited result')
  expect(kinds.stdout.delta).toBe('builtin\nregistered\nfunction\nfile\n')
  expect(kinds.audit).toEqual([])

  const invalid = await env.exec({ shellId: descriptions.shellId, script: 'type -z cd' })
  expect(invalid.status).toBe('exited')
  if (invalid.status !== 'exited') throw new Error('expected exited result')
  expect(invalid.exitCode).toBe(2)
  expect(invalid.stderr.delta).toContain('unsupported option')
  expect(invalid.audit).toEqual([])
})

test('BashEnvironment scopes registered command storage by agent session with shell fallback', async () => {
  const registry = new CommandRegistry()
  registry.register(counterCommand())
  let nextShell = 0
  const root = await mkdtemp(join(tmpdir(), 'demi-command-storage-'))
  const env = new BashEnvironment({
    host: new LocalHost(root, { storeRoot: join(root, '.host-store') }),
    commands: registry,
    shellIdFactory: () => `shell-${++nextShell}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'counter inc' })
  const second = await env.exec({ script: 'counter inc' })
  const third = await env.exec({ shellId: first.shellId, script: 'counter inc' })
  const agentFirst = await env.exec({ agentSessionId: 'agent-a', script: 'counter inc' })
  const agentSecond = await env.exec({ agentSessionId: 'agent-a', script: 'counter inc' })
  const otherAgent = await env.exec({ agentSessionId: 'agent-b', script: 'counter inc' })

  expect(first.stdout.delta).toBe('1\n')
  expect(second.stdout.delta).toBe('1\n')
  expect(third.stdout.delta).toBe('2\n')
  expect(agentFirst.stdout.delta).toBe('1\n')
  expect(agentSecond.stdout.delta).toBe('2\n')
  expect(otherAgent.stdout.delta).toBe('1\n')
})

test('BashEnvironment passes heredoc content to registered commands via stdinField', async () => {
  const registry = new CommandRegistry()
  registry.register(stdinCommand())
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    commands: registry,
    shellIdFactory: () => 'shell-heredoc-registered',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: "stdin-cmd write <<'EOF'\nhello $NAME\nEOF",
  })

  expect(result.status).toBe('exited')
  expect(result.stdout.delta).toBe('hello $NAME\n')
})

test('BashEnvironment passes heredoc content to system commands and expands unquoted variables', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-heredoc-system',
    initialEnv: { PATH: process.env.PATH ?? '', NAME: 'Alice' },
  })

  const result = await env.exec({
    script: 'cat <<EOF\nHello, $NAME\nEOF',
  })

  expect(result.status).toBe('exited')
  expect(result.stdout.delta).toBe('Hello, Alice\n')
})

test('BashEnvironment expands unquoted glob arguments through the host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-glob-'))
  await mkdir(join(root, 'src'))
  await writeFile(join(root, 'alpha.ts'), '')
  await writeFile(join(root, 'beta.ts'), '')
  await writeFile(join(root, '.hidden.ts'), '')
  await writeFile(join(root, 'src', 'one.ts'), '')
  await writeFile(join(root, 'src', 'two.ts'), '')
  await writeFile(join(root, 'src', 'skip.js'), '')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-glob',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const args = await env.exec({ script: 'printf "%s|" *.ts' })
  expect(args.status).toBe('exited')
  if (args.status !== 'exited') throw new Error('expected exited result')
  expect(args.stdout.delta).toBe('alpha.ts|beta.ts|')

  const quoted = await env.exec({ shellId: args.shellId, script: 'printf "%s|" "*.ts" \\*.ts missing-*.ts' })
  expect(quoted.stdout.delta).toBe('*.ts|*.ts|missing-*.ts|')

  const loop = await env.exec({
    shellId: args.shellId,
    script: 'for file in src/*.ts; do printf "<%s>" "$file"; done',
  })
  expect(loop.stdout.delta).toBe('<src/one.ts><src/two.ts>')
})

test('BashEnvironment applies file redirections without handing scripts to a system shell', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-redir-'))
  await writeFile(join(root, 'input.txt'), 'from-file')
  const registry = new CommandRegistry()
  registry.register(echoCommand())
  const env = new BashEnvironment({
    host: new LocalHost(root),
    commands: registry,
    shellIdFactory: () => 'shell-redirections',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const write = await env.exec({ script: 'printf one > out.txt' })
  expect(write.stdout.delta).toBe('')
  const append = await env.exec({ shellId: write.shellId, script: 'printf two >> out.txt' })
  expect(append.stdout.delta).toBe('')
  const readBack = await env.exec({ shellId: write.shellId, script: 'cat out.txt' })
  expect(readBack.stdout.delta).toBe('onetwo')

  const blocked = await env.exec({
    shellId: write.shellId,
    script: 'set -C; sh -c "printf ran > marker.txt" > out.txt',
  })
  expect(blocked.status).toBe('exited')
  if (blocked.status !== 'exited') throw new Error('expected exited result')
  expect(blocked.exitCode).toBe(1)
  expect(blocked.stderr.delta).toContain('cannot overwrite existing file')
  await expect(access(join(root, 'marker.txt'))).rejects.toThrow()
  await expect(readFile(join(root, 'out.txt'), 'utf8')).resolves.toBe('onetwo')

  const forced = await env.exec({ shellId: write.shellId, script: 'printf forced >| out.txt' })
  expect(forced.status).toBe('exited')
  if (forced.status !== 'exited') throw new Error('expected exited result')
  expect(forced.exitCode).toBe(0)
  await expect(readFile(join(root, 'out.txt'), 'utf8')).resolves.toBe('forced')

  const appendWithNoclobber = await env.exec({ shellId: write.shellId, script: 'printf append >> out.txt' })
  expect(appendWithNoclobber.status).toBe('exited')
  if (appendWithNoclobber.status !== 'exited') throw new Error('expected exited result')
  expect(appendWithNoclobber.exitCode).toBe(0)
  await expect(readFile(join(root, 'out.txt'), 'utf8')).resolves.toBe('forcedappend')

  const blockedShortcut = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" &> out.txt',
  })
  expect(blockedShortcut.status).toBe('exited')
  if (blockedShortcut.status !== 'exited') throw new Error('expected exited result')
  expect(blockedShortcut.exitCode).toBe(1)
  expect(blockedShortcut.stderr.delta).toContain('cannot overwrite existing file')

  const restoredClobber = await env.exec({ shellId: write.shellId, script: 'set +C; printf restored > out.txt' })
  expect(restoredClobber.status).toBe('exited')
  if (restoredClobber.status !== 'exited') throw new Error('expected exited result')
  expect(restoredClobber.exitCode).toBe(0)
  await expect(readFile(join(root, 'out.txt'), 'utf8')).resolves.toBe('restored')

  const stdin = await env.exec({ shellId: write.shellId, script: 'cat < input.txt' })
  expect(stdin.stdout.delta).toBe('from-file')

  const stderr = await env.exec({ shellId: write.shellId, script: 'sh -c "printf err >&2" 2> err.txt' })
  expect(stderr.stderr.delta).toBe('')
  const errBack = await env.exec({ shellId: write.shellId, script: 'cat err.txt' })
  expect(errBack.stdout.delta).toBe('err')

  const duplicated = await env.exec({ shellId: write.shellId, script: 'sh -c "printf err >&2" 2>&1' })
  expect(duplicated.stdout.delta).toBe('err')
  expect(duplicated.stderr.delta).toBe('')

  const bothToFile = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" > both.txt 2>&1',
  })
  expect(bothToFile.stdout.delta).toBe('')
  expect(bothToFile.stderr.delta).toBe('')
  const bothBack = await env.exec({ shellId: write.shellId, script: 'cat both.txt' })
  expect(bothBack.stdout.delta).toBe('outerr')

  const bothShortcut = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" &> shortcut.txt',
  })
  expect(bothShortcut.stdout.delta).toBe('')
  expect(bothShortcut.stderr.delta).toBe('')
  const shortcutBack = await env.exec({ shellId: write.shellId, script: 'cat shortcut.txt' })
  expect(shortcutBack.stdout.delta).toBe('outerr')

  const bothAppend = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf more; printf err >&2" &>> shortcut.txt',
  })
  expect(bothAppend.stdout.delta).toBe('')
  expect(bothAppend.stderr.delta).toBe('')
  const appendedShortcutBack = await env.exec({ shellId: write.shellId, script: 'cat shortcut.txt' })
  expect(appendedShortcutBack.stdout.delta).toBe('outerrmoreerr')

  const duplicateBeforeFile = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" 2>&1 > stdout-only.txt',
  })
  expect(duplicateBeforeFile.stdout.delta).toBe('err')
  expect(duplicateBeforeFile.stderr.delta).toBe('')
  const stdoutOnlyBack = await env.exec({ shellId: write.shellId, script: 'cat stdout-only.txt' })
  expect(stdoutOnlyBack.stdout.delta).toBe('out')

  const shortcutBeforeFile = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" &> shortcut-order.txt > shortcut-stdout.txt',
  })
  expect(shortcutBeforeFile.stdout.delta).toBe('')
  expect(shortcutBeforeFile.stderr.delta).toBe('')
  const shortcutOrderBack = await env.exec({ shellId: write.shellId, script: 'cat shortcut-order.txt' })
  expect(shortcutOrderBack.stdout.delta).toBe('err')
  const shortcutStdoutBack = await env.exec({ shellId: write.shellId, script: 'cat shortcut-stdout.txt' })
  expect(shortcutStdoutBack.stdout.delta).toBe('out')

  const closedStderr = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" 2>&-',
  })
  expect(closedStderr.stdout.delta).toBe('out')
  expect(closedStderr.stderr.delta).toBe('')

  const closedStdout = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" 1>&-',
  })
  expect(closedStdout.stdout.delta).toBe('')
  expect(closedStdout.stderr.delta).toBe('err')

  const duplicateThenClose = await env.exec({
    shellId: write.shellId,
    script: 'sh -c "printf out; printf err >&2" 2>&1 1>&-',
  })
  expect(duplicateThenClose.stdout.delta).toBe('err')
  expect(duplicateThenClose.stderr.delta).toBe('')

  const registered = await env.exec({ shellId: write.shellId, script: 'echo-cmd run --value ok > registered.txt' })
  expect(registered.stdout.delta).toBe('')
  const registeredBack = await env.exec({ shellId: write.shellId, script: 'cat registered.txt' })
  expect(registeredBack.stdout.delta).toBe('registered:ok\n')

  const largeContent = 'x'.repeat(256 * 1024)
  const large = await env.exec({
    shellId: write.shellId,
    script: String.raw`sh -c "yes x | tr -d '\n' | head -c 262144; sleep 0.03" > large.txt`,
    timeoutMs: 1,
    maxOutputBytes: 512 * 1024,
  })
  expect(large.status).toBe('running')
  if (large.status !== 'running') throw new Error('expected running result')
  expect(large.stdout.delta).toBe('')

  const largeDone = await waitForCommand(env, large.commandId, {
    status: 'exited',
    maxOutputBytes: 512 * 1024,
    timeoutMs: 10_000,
  })
  expect(largeDone.status).toBe('exited')
  if (largeDone.status !== 'exited') throw new Error('expected exited result')
  expect(largeDone.exitCode).toBe(0)
  expect(largeDone.stdout.delta).toBe('')
  expect(largeDone.stderr.delta).toBe('')
  await expect(readFile(join(root, 'large.txt'), 'utf8')).resolves.toBe(largeContent)
})

test('BashEnvironment executes simple pipelines without handing scripts to a system shell', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-pipeline-'))
  await writeFile(join(root, 'input.txt'), 'alpha\nbeta\n')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-pipeline',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: 'cat input.txt | grep beta | tr a-z A-Z' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('BETA\n')

  const stderr = await env.exec({ shellId: result.shellId, script: 'sh -c "printf err >&2; printf out" | cat' })
  expect(stderr.stdout.delta).toBe('out')
  expect(stderr.stderr.delta).toBe('err')
})

test('BashEnvironment routes portable file commands through Host.fs before Host.process.spawn', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-portable-host-fs-'))
  await writeFile(join(root, 'input.txt'), 'from host fs\n')
  const host = new NoSpawnLocalHost(root)
  const env = new BashEnvironment({
    host,
    shellIdFactory: () => 'shell-portable-host-fs',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: 'cat input.txt | tee copied.txt\ncat copied.txt' })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe('from host fs\nfrom host fs\n')
  expect(host.processSpawnCalls).toBe(0)
  expect(result.audit).toEqual([
    {
      kind: 'portable-command',
      name: 'cat',
      args: ['input.txt'],
      cwd: root,
      exitCode: 0,
    },
    {
      kind: 'portable-command',
      name: 'tee',
      args: ['copied.txt'],
      cwd: root,
      exitCode: 0,
    },
    {
      kind: 'portable-command',
      name: 'cat',
      args: ['copied.txt'],
      cwd: root,
      exitCode: 0,
    },
  ])
})

test('BashEnvironment executes compound commands inside pipelines', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-compound-pipeline-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-compound-pipeline',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const group = await env.exec({ script: '{ printf "left"; } | cat' })
  expect(group.status).toBe('exited')
  expect(group.stdout.delta).toBe('left')

  const ifPipeline = await env.exec({
    shellId: group.shellId,
    script: 'if true; then printf "yes"; fi | cat',
  })
  expect(ifPipeline.stdout.delta).toBe('yes')

  const forPipeline = await env.exec({
    shellId: group.shellId,
    script: 'for item in a b; do printf "$item"; done | cat',
  })
  expect(forPipeline.stdout.delta).toBe('ab')

  const whilePipeline = await env.exec({
    shellId: group.shellId,
    script: 'printf "one\\ntwo\\n" | while read line; do printf "<%s>" "$line"; done',
  })
  expect(whilePipeline.stdout.delta).toBe('<one><two>')

  const redirected = await env.exec({
    shellId: group.shellId,
    script: '{ printf hidden > file.txt; } | cat; printf "|"; cat file.txt',
  })
  expect(redirected.stdout.delta).toBe('|hidden')
})

test('BashEnvironment supports negated pipelines and commands', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-negated',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const trueResult = await env.exec({ script: '! true' })
  expect(trueResult.status).toBe('exited')
  if (trueResult.status !== 'exited') throw new Error('expected exited result')
  expect(trueResult.exitCode).toBe(1)

  const status = await env.exec({ shellId: trueResult.shellId, script: 'printf "$?"' })
  expect(status.stdout.delta).toBe('1')

  const list = await env.exec({ shellId: trueResult.shellId, script: '! false && printf yes; ! true || printf no' })
  expect(list.stdout.delta).toBe('yesno')

  const pipeline = await env.exec({ shellId: trueResult.shellId, script: '! printf alpha | grep beta; printf "$?"' })
  expect(pipeline.stdout.delta).toBe('0')

  const running = await env.exec({
    shellId: trueResult.shellId,
    script: '! sh -c "sleep 0.03; exit 0"',
    timeoutMs: 1,
  })
  expect(running.status).toBe('running')
  if (running.status !== 'running') throw new Error('expected running result')

  const waited = await waitForCommand(env, running.commandId, { status: 'exited' })
  expect(waited.status).toBe('exited')
  if (waited.status !== 'exited') throw new Error('expected exited result')
  expect(waited.exitCode).toBe(1)
  expect(waited.audit).toMatchObject([{ kind: 'system-command', name: 'sh', exitCode: 0 }])

  const background = await env.exec({ shellId: trueResult.shellId, script: '! sh -c "exit 5" &' })
  expect(background.status).toBe('exited')
  if (background.status !== 'exited') throw new Error('expected exited result')
  expect(background.exitCode).toBe(0)
  const waitedBackground = await env.exec({ shellId: trueResult.shellId, script: 'wait %1' })
  expect(waitedBackground.status).toBe('exited')
  if (waitedBackground.status !== 'exited') throw new Error('expected exited result')
  expect(waitedBackground.exitCode).toBe(5)
})

test('BashEnvironment supports if compound commands in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-if',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const thenBranch = await env.exec({ script: 'if true; then printf yes; else printf no; fi; printf ":$?"' })
  expect(thenBranch.status).toBe('exited')
  if (thenBranch.status !== 'exited') throw new Error('expected exited result')
  expect(thenBranch.exitCode).toBe(0)
  expect(thenBranch.stdout.delta).toBe('yes:0')

  const elseBranch = await env.exec({ shellId: thenBranch.shellId, script: 'if false; then printf no; else printf else; fi' })
  expect(elseBranch.status).toBe('exited')
  if (elseBranch.status !== 'exited') throw new Error('expected exited result')
  expect(elseBranch.stdout.delta).toBe('else')
  expect(elseBranch.exitCode).toBe(0)

  const elifBranch = await env.exec({
    shellId: thenBranch.shellId,
    script: 'if false; then printf no; elif true; then printf elif; false; else printf else; fi; printf ":$?"',
  })
  expect(elifBranch.status).toBe('exited')
  if (elifBranch.status !== 'exited') throw new Error('expected exited result')
  expect(elifBranch.stdout.delta).toBe('elif:1')
  expect(elifBranch.exitCode).toBe(0)

  const noMatch = await env.exec({ shellId: thenBranch.shellId, script: 'if false; then printf no; fi; printf "no_match:$?"' })
  expect(noMatch.stdout.delta).toBe('no_match:0')

  const conditionList = await env.exec({
    shellId: thenBranch.shellId,
    script: 'if false; true; then X=from-if; fi; printf "$X"',
  })
  expect(conditionList.stdout.delta).toBe('from-if')

  const negated = await env.exec({ shellId: thenBranch.shellId, script: '! if true; then false; fi; printf "$?"' })
  expect(negated.stdout.delta).toBe('0')
})

test('BashEnvironment supports common double-bracket conditional commands', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-conditional-'))
  await mkdir(join(root, 'pkg'))
  await writeFile(join(root, 'input.txt'), 'data')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-conditional',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script:
      'name=alpha; if [[ -f input.txt && "$name" == a* ]]; then printf yes; else printf no; fi; [[ 4 -gt 2 ]]; printf "|$?"; [[ ! -d missing ]]; printf "|$?"; [[ "abc" =~ ^a ]]; printf "|$?"; [[ "$name" == [ab]* ]]; printf "|$?"; [[ "$name" == [!z]* ]]; printf "|$?"',
  })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.stdout.delta).toBe('yes|0|0|0|0|0')
  expect(result.audit.some((event) => event.kind === 'system-command' && event.name === 'test')).toBe(false)

  const quotedPattern = await env.exec({ shellId: result.shellId, script: '[[ "$name" == "a*" ]]; printf "$?"' })
  expect(quotedPattern.stdout.delta).toBe('1')
})

test('BashEnvironment supports case compound commands in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-case-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-case',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const matched = await env.exec({
    script: 'kind=demo.ts; case "$kind" in *.ts|*.tsx) printf ts ;; *.md) printf md ;; *) printf other ;; esac; printf "|$?"',
  })
  expect(matched.status).toBe('exited')
  if (matched.status !== 'exited') throw new Error('expected exited result')
  expect(matched.stdout.delta).toBe('ts|0')

  const quotedPattern = await env.exec({
    shellId: matched.shellId,
    script: 'kind=alpha; case "$kind" in "a*") printf literal ;; a*) printf pattern ;; esac',
  })
  expect(quotedPattern.stdout.delta).toBe('pattern')

  const characterClass = await env.exec({
    shellId: matched.shellId,
    script: 'kind=beta; case "$kind" in [ab]*) printf class ;; *) printf other ;; esac',
  })
  expect(characterClass.stdout.delta).toBe('class')

  const fallthrough = await env.exec({
    shellId: matched.shellId,
    script: 'case x in x) printf one ;& y) printf two ;; esac; printf "|"; case x in x) printf match ;;& x) printf again ;; esac',
  })
  expect(fallthrough.stdout.delta).toBe('onetwo|matchagain')

  const unmatched = await env.exec({ shellId: matched.shellId, script: 'case x in y) false ;; esac; printf "$?"' })
  expect(unmatched.stdout.delta).toBe('0')

  const redirected = await env.exec({
    shellId: matched.shellId,
    script: 'case foo in f*) printf ok ;; esac > case.txt; printf "$(cat case.txt)"',
  })
  expect(redirected.stdout.delta).toBe('ok')

  const negated = await env.exec({ shellId: matched.shellId, script: '! case x in x) false ;; esac; printf "$?"' })
  expect(negated.stdout.delta).toBe('0')

  const loopControl = await env.exec({
    shellId: matched.shellId,
    script: 'for item in a b; do case "$item" in a) continue ;; esac; printf "$item"; done; printf "|$?"',
  })
  expect(loopControl.stdout.delta).toBe('b|0')
})

test('BashEnvironment does not negate an explicit exit inside if', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-if-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: '! if true; then exit 7; fi' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(7)
})

test('BashEnvironment supports finite for loops in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-for',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const words = await env.exec({
    script: 'for item in a b; do printf "$item"; false; done; printf "|$item|$?"',
  })
  expect(words.status).toBe('exited')
  if (words.status !== 'exited') throw new Error('expected exited result')
  expect(words.exitCode).toBe(0)
  expect(words.stdout.delta).toBe('ab|b|1')

  const positionals = await env.exec({
    shellId: words.shellId,
    script: 'set -- x y; for item; do printf "$item"; done; printf "|$item|$?"',
  })
  expect(positionals.stdout.delta).toBe('xy|y|0')

  const empty = await env.exec({
    shellId: words.shellId,
    script: 'EMPTY=before; for EMPTY in; do printf nope; done; printf "$EMPTY|$?"',
  })
  expect(empty.stdout.delta).toBe('before|0')

  const negated = await env.exec({
    shellId: words.shellId,
    script: '! for item in a b; do false; done; printf "$?"',
  })
  expect(negated.stdout.delta).toBe('0')
})

test('BashEnvironment supports C-style for loops in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-c-for-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-c-for',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const counted = await env.exec({ script: 'for ((i=0; i<3; i++)); do printf "$i"; done; printf "|%s:%s" "$i" "$?"' })
  expect(counted.status).toBe('exited')
  if (counted.status !== 'exited') throw new Error('expected exited result')
  expect(counted.exitCode).toBe(0)
  expect(counted.stdout.delta).toBe('012|3:0')

  const continued = await env.exec({
    shellId: counted.shellId,
    script: 'for ((; i<6; i++)); do if (( i == 4 )); then continue; fi; printf "$i"; done; printf "|%s:%s" "$i" "$?"',
  })
  expect(continued.stdout.delta).toBe('35|6:0')

  const broken = await env.exec({ shellId: counted.shellId, script: 'for ((i=0; i<3; i++)); do break; done; printf "%s:%s" "$i" "$?"' })
  expect(broken.stdout.delta).toBe('0:0')

  const redirected = await env.exec({
    shellId: counted.shellId,
    script: 'for ((i=0; i<2; i++)); do printf "$i"; done > out.txt; cat out.txt',
  })
  expect(redirected.stdout.delta).toBe('01')

  const negated = await env.exec({ shellId: counted.shellId, script: '! for ((i=0; i<1; i++)); do false; done; printf "$?"' })
  expect(negated.stdout.delta).toBe('0')
})

test('BashEnvironment does not negate an explicit exit inside a for loop', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-for-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: '! for item in a; do exit 7; done' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(7)
})

test('BashEnvironment supports finite while and until loops in the current shell session', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-loops',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const whileLoop = await env.exec({
    script: 'i=0; while test "$i" -lt 2; do printf "$i"; i=$(expr "$i" + 1); false; done; printf "|$i|$?"',
  })
  expect(whileLoop.status).toBe('exited')
  if (whileLoop.status !== 'exited') throw new Error('expected exited result')
  expect(whileLoop.exitCode).toBe(0)
  expect(whileLoop.stdout.delta).toBe('01|2|1')

  const untilLoop = await env.exec({
    shellId: whileLoop.shellId,
    script: 'i=0; until test "$i" -ge 2; do printf "$i"; i=$(expr "$i" + 1); false; done; printf "|$i|$?"',
  })
  expect(untilLoop.stdout.delta).toBe('01|2|1')

  const emptyWhile = await env.exec({ shellId: whileLoop.shellId, script: 'while false; do printf nope; done; printf "$?"' })
  expect(emptyWhile.stdout.delta).toBe('0')

  const emptyUntil = await env.exec({ shellId: whileLoop.shellId, script: 'until true; do printf nope; done; printf "$?"' })
  expect(emptyUntil.stdout.delta).toBe('0')

  const negated = await env.exec({
    shellId: whileLoop.shellId,
    script: '! while false; do printf nope; done; printf "$?"; ! until true; do printf nope; done; printf "|$?"',
  })
  expect(negated.stdout.delta).toBe('1|1')
})

test('BashEnvironment does not negate an explicit exit inside a loop', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-loop-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: '! while true; do exit 7; done' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(7)
})

test('BashEnvironment supports break and continue inside loops', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-loop-control',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const broken = await env.exec({
    script: 'for item in a b c; do printf "$item"; break; printf no; done; printf "|$item|$?"',
  })
  expect(broken.status).toBe('exited')
  if (broken.status !== 'exited') throw new Error('expected exited result')
  expect(broken.exitCode).toBe(0)
  expect(broken.stdout.delta).toBe('a|a|0')

  const continued = await env.exec({
    shellId: broken.shellId,
    script: 'for item in a b; do printf "$item"; continue; printf no; done; printf "|$item|$?"',
  })
  expect(continued.stdout.delta).toBe('ab|b|0')

  const nestedBreak = await env.exec({
    shellId: broken.shellId,
    script: 'for i in 1 2; do for j in a b; do printf "$i$j"; break 2; done; printf after; done; printf "|$?"',
  })
  expect(nestedBreak.stdout.delta).toBe('1a|0')

  const nestedContinue = await env.exec({
    shellId: broken.shellId,
    script: 'for i in 1 2; do for j in a b; do printf "$i$j"; continue 2; printf no; done; printf after; done; printf "|$?"',
  })
  expect(nestedContinue.stdout.delta).toBe('1a2a|0')

  const whileContinue = await env.exec({
    shellId: broken.shellId,
    script:
      'i=0; while test "$i" -lt 3; do i=$(expr "$i" + 1); if test "$i" = 2; then continue; fi; printf "$i"; done; printf "|$i|$?"',
  })
  expect(whileContinue.stdout.delta).toBe('13|3|0')
})

test('BashEnvironment validates break and continue usage', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-loop-control-errors',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const outside = await env.exec({ script: 'break' })
  expect(outside.status).toBe('exited')
  if (outside.status !== 'exited') throw new Error('expected exited result')
  expect(outside.exitCode).toBe(0)
  expect(outside.stderr.delta).toContain('only meaningful')
  expect(outside.audit).toEqual([])

  const invalidCount = await env.exec({ shellId: outside.shellId, script: 'for item in a; do continue 0; done' })
  expect(invalidCount.status).toBe('exited')
  if (invalidCount.status !== 'exited') throw new Error('expected exited result')
  expect(invalidCount.exitCode).toBe(0)
  expect(invalidCount.stderr.delta).toContain('loop count out of range')

  const nonNumeric = await env.exec({ shellId: outside.shellId, script: 'for item in a; do break bad; done' })
  expect(nonNumeric.status).toBe('exited')
  if (nonNumeric.status !== 'exited') throw new Error('expected exited result')
  expect(nonNumeric.exitCode).toBe(128)
  expect(nonNumeric.stderr.delta).toContain('numeric argument required')
})

test('BashEnvironment supports group commands in the current shell session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-group-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-group',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: '{ X=group; cd pkg; false; }; printf "%s|%s|%s" "$X" "$PWD" "$?"',
  })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe(`group|${join(root, 'pkg')}|1`)

  const negated = await env.exec({ shellId: result.shellId, script: '! { false; }; printf "$?"' })
  expect(negated.stdout.delta).toBe('0')
})

test('BashEnvironment applies output redirections to compound commands', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-compound-redir-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-compound-redir',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const group = await env.exec({
    script: '{ printf out; printf err >&2; } > combined.txt 2>&1; printf "$(cat combined.txt)"',
  })
  expect(group.status).toBe('exited')
  if (group.status !== 'exited') throw new Error('expected exited result')
  expect(group.stdout.delta).toBe('outerr')
  expect(group.stderr.delta).toBe('')

  const ifResult = await env.exec({
    shellId: group.shellId,
    script: 'if true; then printf yes; fi > if.txt; printf "$(cat if.txt)"',
  })
  expect(ifResult.stdout.delta).toBe('yes')

  const forResult = await env.exec({
    shellId: group.shellId,
    script: 'for item in a b; do printf "$item"; done > loop.txt; (printf sub) >> loop.txt; printf "$(cat loop.txt)"',
  })
  expect(forResult.stdout.delta).toBe('absub')

  const whileResult = await env.exec({
    shellId: group.shellId,
    script: 'i=0; while test "$i" -lt 2; do printf "$i"; i=$(expr "$i" + 1); done > while.txt; printf "$(cat while.txt)"',
  })
  expect(whileResult.stdout.delta).toBe('01')

  const stderrOnly = await env.exec({
    shellId: group.shellId,
    script: '{ printf out; printf err >&2; } 1>&2; printf done',
  })
  expect(stderrOnly.stdout.delta).toBe('done')
  expect(stderrOnly.stderr.delta).toBe('outerr')
})

test('BashEnvironment applies input redirections to compound commands', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-compound-input-redir-'))
  await writeFile(join(root, 'input.txt'), 'data')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-compound-input-redir',
    initialEnv: { PATH: process.env.PATH ?? '', NAME: 'world' },
  })
  env.registerCommand(stdinCommand())

  const group = await env.exec({ script: '{ true; cat; printf "|"; cat; } < input.txt' })
  expect(group.status).toBe('exited')
  if (group.status !== 'exited') throw new Error('expected exited result')
  expect(group.stdout.delta).toBe('data|')

  const ifResult = await env.exec({ shellId: group.shellId, script: 'if true; then cat; fi < input.txt' })
  expect(ifResult.stdout.delta).toBe('data')

  const whileResult = await env.exec({
    shellId: group.shellId,
    script: 'i=0; while (( i < 1 )); do cat; (( i++ )); done < input.txt',
  })
  expect(whileResult.stdout.delta).toBe('data')

  const subshell = await env.exec({ shellId: group.shellId, script: '(cat; cd /) < input.txt; printf "|$PWD"' })
  expect(subshell.stdout.delta).toBe(`data|${root}`)

  const heredoc = await env.exec({
    shellId: group.shellId,
    script: "{ cat; } <<'EOF'\nhello $NAME\nEOF",
  })
  expect(heredoc.stdout.delta).toBe('hello $NAME\n')

  const hereString = await env.exec({ shellId: group.shellId, script: '{ cat; } <<< "$NAME"' })
  expect(hereString.stdout.delta).toBe('world\n')

  const registered = await env.exec({
    shellId: group.shellId,
    script: '{ stdin-cmd write; printf "|"; stdin-cmd write; } < input.txt',
  })
  expect(registered.stdout.delta).toBe('data|')
})

test('BashEnvironment supports subshell commands without leaking session state', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-subshell-'))
  await mkdir(join(root, 'pkg'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-subshell',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: '(X=sub; cd pkg; printf "%s|%s|" "$X" "$PWD"; false); printf "%s|%s|%s" "$X" "$PWD" "$?"',
  })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(0)
  expect(result.stdout.delta).toBe(`sub|${join(root, 'pkg')}||${root}|1`)

  const negated = await env.exec({ shellId: result.shellId, script: '! (false); printf "$?"; (exit 7); printf "|$?"; ! (exit 7); printf "|$?"' })
  expect(negated.stdout.delta).toBe('0|7|0')
})

test('BashEnvironment does not negate an explicit exit inside a group command', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-group-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: '! { exit 7; }' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(7)
})

test('BashEnvironment does not negate an explicit exit builtin', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-negated-exit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: '! exit 7' })
  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(7)
})

test('BashEnvironment rejects unsupported parser constructs instead of handing them to system shell', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-unsupported',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  await expect(env.exec({ script: 'time printf SHOULD_NOT_RUN' })).rejects.toThrow('Unsupported shell syntax: timed pipelines')
})

test('BashEnvironment records system command audit events', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-system',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  // printf is a portable command now (see DEMI_PORTABLE_COMMANDS), so it no
  // longer exercises the system-command (real spawn) audit path this test is
  // actually about — node is in SYSTEM_TOOL_NAMES and never portable, so it
  // stays a reliable stand-in for "a real, non-portable system command".
  const result = await env.exec({ script: `node -e "process.stdout.write('hi')"` })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.stdout.delta).toBe('hi')
  expect(result.audit).toEqual([
    {
      kind: 'system-command',
      name: 'node',
      args: ['-e', "process.stdout.write('hi')"],
      cwd: process.cwd(),
      exitCode: 0,
    },
  ])
})

test('BashEnvironment reports system command spawn failures without hanging', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-spawn-failure',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: 'demi-command-that-does-not-exist', timeoutMs: 1 })

  expect(result.status).toBe('exited')
  if (result.status !== 'exited') throw new Error('expected exited result')
  expect(result.exitCode).toBe(127)
  expect(result.stderr.delta).toContain('demi-command-that-does-not-exist')
})

test('BashEnvironment supports running/yield then shell_status', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-wait',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'sh -c "sleep 0.03; printf done"', timeoutMs: 1 })

  expect(first.status).toBe('running')
  const second = await waitForCommand(env, first.commandId, { status: 'exited' })
  expect(second.status).toBe('exited')
  if (second.status !== 'exited') throw new Error('expected exited result')
  expect(second.stdout.delta).toBe('done')
  expect(second.audit).toMatchObject([{ kind: 'system-command', name: 'sh' }])
  const status = await env.exec({ shellId: first.shellId, script: 'printf "$?"' })
  expect(status.stdout.delta).toBe('0')
})

test('BashEnvironment runs shell_exec without shellId in an auxiliary shell when the default shell is busy', async () => {
  let nextShell = 0
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => `shell-busy-default-${++nextShell}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const foreground = await env.exec({
    agentSessionId: 'agent-busy-default',
    script: 'sh -c "sleep 10"',
    timeoutMs: 1,
  })
  expect(foreground.status).toBe('running')
  if (foreground.status !== 'running') throw new Error('expected running result')
  expect(foreground.shellId).toBe('shell-busy-default-1')

  const probe = await env.exec({
    agentSessionId: 'agent-busy-default',
    script: 'printf probe',
  })

  expect(probe.status).toBe('exited')
  if (probe.status !== 'exited') throw new Error('expected exited result')
  expect(probe.shellId).toBe('shell-busy-default-2')
  expect(probe.stdout.delta).toBe('probe')

  const aborted = await env.abort({ commandId: foreground.commandId })
  expect(aborted.status).toBe('aborted')

  const reusedDefault = await env.exec({
    agentSessionId: 'agent-busy-default',
    script: 'printf default',
  })
  expect(reusedDefault.status).toBe('exited')
  expect(reusedDefault.shellId).toBe('shell-busy-default-1')
  expect(reusedDefault.stdout.delta).toBe('default')
})

test('BashEnvironment returns bounded output deltas by maxOutputBytes', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-output-limit',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c "printf 1234567890; sleep 0.03; printf done"',
    timeoutMs: 1,
    maxOutputBytes: 5,
  })

  expect(first.status).toBe('running')
  if (first.status !== 'running') throw new Error('expected running result')
  // First bounded read: stop as soon as we observe a non-empty truncated chunk.
  const firstChunk = await waitForCommand(env, first.commandId, {
    maxOutputBytes: 5,
    predicate: (result, acc) => acc.stdout.length > 0 || result.status === 'exited',
  })
  expect(firstChunk.stdout.delta).toBe('12345')
  expect(firstChunk.stdout.truncated).toBe(true)

  // Remaining bytes may arrive across several status polls while still running — accumulate.
  const second = await waitForCommand(env, first.commandId, {
    status: 'exited',
    maxOutputBytes: 20,
  })
  expect(second.status).toBe('exited')
  if (second.status !== 'exited') throw new Error('expected exited result')
  expect(second.stdout.delta).toBe('67890done')
})

test('BashEnvironment records visible stdout and stderr in arrival order', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-output-order',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: 'sh -c "printf out1; sleep 0.03; printf err1 >&2; sleep 0.03; printf out2; sleep 0.03; printf err2 >&2"',
    timeoutMs: 1_000,
  })

  expect(result.status).toBe('exited')
  expect(result.stdout.delta).toBe('out1out2')
  expect(result.stderr.delta).toBe('err1err2')
  expect(result.output.chunks).toEqual([
    { stream: 'stdout', text: 'out1' },
    { stream: 'stderr', text: 'err1' },
    { stream: 'stdout', text: 'out2' },
    { stream: 'stderr', text: 'err2' },
  ])
  expect(result.output.text).toBe('out1err1out2err2')
})

test('BashEnvironment output preview includes stdout from sequential foreground commands', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({
    script: 'echo SHORT-OUT; echo SHORT-ERR >&2',
    timeoutMs: 1_000,
  })

  expect(result.status).toBe('exited')
  expect(result.stdout.delta).toBe('SHORT-OUT\n')
  expect(result.stderr.delta).toBe('SHORT-ERR\n')
  expect(result.output.chunks).toEqual([
    { stream: 'stdout', text: 'SHORT-OUT\n' },
    { stream: 'stderr', text: 'SHORT-ERR\n' },
  ])
  expect(result.output.text).toBe('SHORT-OUT\nSHORT-ERR\n')
})

test('BashEnvironment exposes complete split command artifacts through /@ and Host.store', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-command-artifact-'))
  const host = new LocalHost(root, { storeRoot: join(root, '.host-store') })
  let nextCommand = 0
  const env = new BashEnvironment({
    host,
    shellIdFactory: () => 'shell-command-artifact',
    commandIdFactory: () => `cmd-artifact-${++nextCommand}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const produced = await env.exec({
    agentSessionId: 'agent-command-artifact',
    script: 'printf "out1\\nout2\\n"; printf "err1\\nerr2\\n" >&2',
    timeoutMs: 1_000,
  })
  expect(produced.status).toBe('exited')
  const commandId = produced.commandId
  expect(produced.stdout.path).toBe(`/@/commands/${commandId}/stdout.txt`)
  expect(produced.stderr.path).toBe(`/@/commands/${commandId}/stderr.txt`)

  const stdout = await env.exec({
    agentSessionId: 'agent-command-artifact',
    script: `cat /@/commands/${commandId}/stdout.txt`,
    timeoutMs: 1_000,
  })
  expect(stdout.stdout.delta).toBe('out1\nout2\n')

  const stderr = await env.exec({
    agentSessionId: 'agent-command-artifact',
    script: `cat /@/commands/${commandId}/stderr.txt`,
    timeoutMs: 1_000,
  })
  expect(stderr.stdout.delta).toBe('err1\nerr2\n')

  const selected = await env.exec({
    agentSessionId: 'agent-command-artifact',
    script:
      `tail -n 1 /@/commands/${commandId}/stdout.txt; ` +
      `grep -n err2 /@/commands/${commandId}/stderr.txt; ` +
      `if cat /@/commands/${commandId}/output.txt >/dev/null 2>&1; then printf bad; else printf missing; fi`,
    timeoutMs: 1_000,
  })
  expect(selected.stdout.delta).toBe('out2\n2:err2\nmissing')

  const meta = await env.exec({
    agentSessionId: 'agent-command-artifact',
    script: `cat /@/commands/${commandId}/meta.json`,
    timeoutMs: 1_000,
  })
  expect(JSON.parse(meta.stdout.delta)).toMatchObject({
    status: 'exited',
    commandId,
    stdout: { path: `/@/commands/${commandId}/stdout.txt`, bytes: 10 },
    stderr: { path: `/@/commands/${commandId}/stderr.txt`, bytes: 10 },
  })

  const persisted = await waitForStoreJson<{ stdout: string; stderr: string }>(
    () => host.store.readJson(`agent-command-artifact/commands/${commandId}/artifact.json`),
  )
  expect(persisted).toMatchObject({
    stdout: 'out1\nout2\n',
    stderr: 'err1\nerr2\n',
  })
})

test('BashEnvironment releaseCommand removes completed command handle and virtual artifacts', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-command-release-'))
  const host = new LocalHost(root, { storeRoot: join(root, '.host-store') })
  let nextCommand = 0
  const env = new BashEnvironment({
    host,
    shellIdFactory: () => 'shell-command-release',
    commandIdFactory: () => `cmd-release-${++nextCommand}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const produced = await env.exec({
    agentSessionId: 'agent-command-release',
    script: 'printf "out\\n"; printf "err\\n" >&2',
    timeoutMs: 1_000,
  })
  expect(produced.status).toBe('exited')
  expect(await env.releaseCommand(produced.commandId)).toBe(true)

  await expect(env.status({ commandId: produced.commandId })).rejects.toThrow('Unknown shell command')

  const artifactProbe = await env.exec({
    agentSessionId: 'agent-command-release',
    script: `if cat /@/commands/${produced.commandId}/stdout.txt >/dev/null 2>&1; then printf found; else printf missing; fi`,
    timeoutMs: 1_000,
  })
  expect(artifactProbe.stdout.delta).toBe('missing')
})

test('BashEnvironment supports shell_write for a foreground process', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-input',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c \'IFS= read -r line; printf %s "$line"\'',
    timeoutMs: 1,
  })
  expect(first.status).toBe('running')

  const written = await env.write({ commandId: first.commandId, stdin: 'typed\n' })
  const stdout =
    written.status === 'exited'
      ? written.stdout.delta
      : (await waitForCommand(env, first.commandId, { status: 'exited' })).stdout.delta
  expect(stdout).toBe('typed')
})

test('BashEnvironment writes shell_write exactly and line readers wait for a newline', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-input-raw-line',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c \'IFS= read -r line; printf %s "$line"\'',
    timeoutMs: 1,
  })
  expect(first.status).toBe('running')

  const withoutNewline = await env.write({ commandId: first.commandId, stdin: 'typed' })
  expect(withoutNewline.status).toBe('running')
  if (withoutNewline.status !== 'running') throw new Error('expected running result')
  expect(withoutNewline.stdout.delta).toBe('')

  const written = await env.write({ commandId: first.commandId, stdin: '\n' })
  const stdout =
    written.status === 'exited'
      ? written.stdout.delta
      : (await waitForCommand(env, first.commandId, { status: 'exited' })).stdout.delta
  expect(stdout).toBe('typed')
})

test('BashEnvironment keeps idle foreground processes running by default', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-idle-default-running',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c \'IFS= read -r line; printf %s "$line"\'',
    timeoutMs: 1,
  })
  expect(first.status).toBe('running')

  const second = await env.status({ commandId: first.commandId })
  expect(second.status).toBe('running')

  const written = await env.write({ commandId: first.commandId, stdin: 'typed\n' })
  const stdout =
    written.status === 'exited'
      ? written.stdout.delta
      : (await waitForCommand(env, first.commandId, { status: 'exited' })).stdout.delta
  expect(stdout).toBe('typed')
})

test('BashEnvironment shell_status is nonblocking for running commands', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-wait-call-window',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c "sleep 0.2; printf done"',
    timeoutMs: 1,
  })
  expect(first.status).toBe('running')

  const waitStartedAt = Date.now()
  const second = await env.status({ commandId: first.commandId })
  expect(second.status).toBe('running')
  expect(Date.now() - waitStartedAt).toBeLessThan(50)

  await env.abort({ commandId: first.commandId })
})

test('BashEnvironment supports shell_abort for a foreground process', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-abort',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'sleep 10', timeoutMs: 1 })
  expect(first.status).toBe('running')

  const aborted = await env.abort({ commandId: first.commandId })
  expect(aborted.status).toBe('aborted')
})

test('BashEnvironment flushes redirected foreground output on shell_abort', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-abort-redir-'))
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-abort-redir',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({
    script: 'sh -c "printf aborted; sleep 10" > aborted.txt',
    timeoutMs: 1_000,
    maxOutputBytes: 1,
  })
  expect(first.status).toBe('running')
  if (first.status !== 'running') throw new Error('expected running result')
  expect(first.stdout.delta).toBe('')

  const aborted = await env.abort({ commandId: first.commandId })
  expect(aborted.status).toBe('aborted')
  expect(aborted.stdout.delta).toBe('')
  expect(aborted.stderr.delta).not.toContain('aborted')
  await expect(readFile(join(root, 'aborted.txt'), 'utf8')).resolves.toBe('aborted')
})

test('BashEnvironment disposeShell kills a foreground process and removes the session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-dispose-foreground-'))
  const leakedPath = join(root, 'foreground-leaked.txt')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-dispose-foreground',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const running = await env.exec({
    script: 'sh -c "sleep 0.2; printf leaked > foreground-leaked.txt"',
    timeoutMs: 1,
  })
  expect(running.status).toBe('running')
  expect(env.getShell(running.shellId)).not.toBeNull()

  expect(await env.disposeShell(running.shellId)).toBe(true)
  expect(await env.disposeShell(running.shellId)).toBe(false)
  expect(env.getShell(running.shellId)).toBeNull()
  await expect(env.exec({ shellId: running.shellId, script: 'printf nope' })).rejects.toThrow('Unknown shell session')

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('BashEnvironment disposeShell kills background jobs and removes the session', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-dispose-background-'))
  const leakedPath = join(root, 'background-leaked.txt')
  const env = new BashEnvironment({
    host: new LocalHost(root),
    shellIdFactory: () => 'shell-dispose-background',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const started = await env.exec({
    script: 'sh -c "sleep 0.2; printf leaked > background-leaked.txt" &',
  })
  expect(started.status).toBe('exited')
  expect(started.stdout.delta).toContain('[1] sh -c')

  expect(await env.disposeShell(started.shellId)).toBe(true)
  expect(env.getShell(started.shellId)).toBeNull()
  await expect(env.exec({ shellId: started.shellId, script: 'jobs' })).rejects.toThrow('Unknown shell session')

  await delay(250)
  await expect(access(leakedPath)).rejects.toThrow()
})

test('BashEnvironment disposeAllShells removes every shell session', async () => {
  let nextSession = 0
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => `shell-dispose-all-${++nextSession}`,
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const first = await env.exec({ script: 'printf one' })
  const second = await env.exec({ script: 'printf two' })
  expect(env.getShell(first.shellId)).not.toBeNull()
  expect(env.getShell(second.shellId)).not.toBeNull()

  await env.disposeAllShells()

  expect(env.getShell(first.shellId)).toBeNull()
  expect(env.getShell(second.shellId)).toBeNull()
})

test('BashEnvironment does not implicitly kill commands after timeoutMs', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-no-timeout',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const result = await env.exec({ script: 'sleep 10', timeoutMs: 1 })

  expect(result.status).toBe('running')
  expect(result.stdout.delta).toBe('')
  const after = await env.status({ commandId: result.commandId })
  expect(after.status).toBe('running')
  await env.abort({ commandId: result.commandId })
})

test('BashEnvironment reuses a shell after abort without leaking abort state', async () => {
  const env = new BashEnvironment({
    host: new LocalHost(process.cwd()),
    shellIdFactory: () => 'shell-reuse-after-stop',
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const running = await env.exec({ script: 'sleep 10', timeoutMs: 1 })
  expect(running.status).toBe('running')
  const aborted = await env.abort({ commandId: running.commandId })
  expect(aborted.status).toBe('aborted')

  const afterAbort = await env.exec({ shellId: running.shellId, script: 'printf after-abort' })
  expect(afterAbort.status).toBe('exited')
  expect(afterAbort.stdout.delta).toBe('after-abort')
})

function echoCommand(): Command {
  return {
    name: 'echo-cmd',
    summary: 'Test registered command.',
    subcommands: [
      {
        name: 'run',
        summary: 'Writes a value.',
        input: {
          value: z.string(),
        },
        examples: ['echo-cmd run --value hello'],
        run: async ({ parsed, io }) => {
          await io.stdout(`registered:${parsed.values.value}\n`)
          return { exitCode: 0, metadata: { echoed: parsed.values.value } }
        },
      },
    ],
  }
}

function counterCommand(): Command {
  return {
    name: 'counter',
    summary: 'Session-scoped counter command.',
    subcommands: [
      {
        name: 'inc',
        summary: 'Increment counter.',
        examples: ['counter inc'],
        run: async ({ io, storage }) => {
          const state = (await storage.readJson<{ count: number }>('counter.json')) ?? { count: 0 }
          state.count += 1
          await storage.writeJson('counter.json', state)
          await io.stdout(`${state.count}\n`)
          return { exitCode: 0 }
        },
      },
    ],
  }
}

function stdinCommand(): Command {
  return {
    name: 'stdin-cmd',
    summary: 'Reads stdin through Command.',
    subcommands: [
      {
        name: 'write',
        summary: 'Writes stdin.',
        input: {
          content: z.string(),
        },
        stdinField: 'content',
        examples: ["stdin-cmd write <<'EOF'\ncontent\nEOF"],
        run: async ({ parsed, io }) => {
          await io.stdout(String(parsed.values.content))
          return { exitCode: 0 }
        },
      },
    ],
  }
}

class NoSpawnLocalHost extends LocalHost {
  processSpawnCalls = 0
  override readonly process: HostProcess = {
    spawn: async (): Promise<never> => {
      this.processSpawnCalls += 1
      throw new Error('Host.process.spawn must not be used')
    },
  }
}

async function waitForStoreJson<T>(read: () => Promise<T | null>): Promise<T> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const value = await read()
    if (value !== null) return value
    await delay(5)
  }
  throw new Error('timed out waiting for Host.store JSON')
}

type ShellStatusResult = Awaited<ReturnType<BashEnvironment['status']>>

/**
 * Poll `env.status` until the command reaches a desired state or a predicate holds.
 * Prefer this over fixed wall-clock sleeps when asserting real OS process lifecycle.
 *
 * Stdout/stderr deltas are accumulated across polls — intermediate status calls would
 * otherwise swallow output that later assertions need.
 */
async function waitForCommand(
  env: BashEnvironment,
  commandId: string,
  options: {
    status?: 'exited' | 'aborted' | 'running'
    maxOutputBytes?: number
    timeoutMs?: number
    /** Evaluated on each poll after accumulating that poll's deltas into `stdout`/`stderr`. */
    predicate?: (result: ShellStatusResult, accumulated: { stdout: string; stderr: string }) => boolean
  } = {},
): Promise<ShellStatusResult> {
  const timeoutMs = options.timeoutMs ?? 3_000
  const startedAt = Date.now()
  let last: ShellStatusResult | undefined
  let stdout = ''
  let stderr = ''
  while (Date.now() - startedAt < timeoutMs) {
    last = await env.status({
      commandId,
      ...(options.maxOutputBytes !== undefined ? { maxOutputBytes: options.maxOutputBytes } : {}),
    })
    stdout += last.stdout.delta
    stderr += last.stderr.delta
    const matched = options.predicate
      ? options.predicate(last, { stdout, stderr })
      : options.status
        ? last.status === options.status
        : last.status === 'exited' || last.status === 'aborted'
    if (matched) {
      return {
        ...last,
        stdout: { ...last.stdout, delta: stdout },
        stderr: { ...last.stderr, delta: stderr },
      }
    }
    await delay(5)
  }
  throw new Error(
    `timed out waiting for command ${commandId}` +
      (options.status ? ` status=${options.status}` : '') +
      `; last=${last?.status ?? 'none'}`,
  )
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

test('BashEnvironment decodes byte-string command output back to UTF-8', async () => {
  // Built-in commands (cat, head, ...) emit stdout as a latin1 byte string for
  // binary transparency. The boundary must decode it back to Unicode, or CJK
  // and emoji surface as mojibake. Regression for the cat/head encoding bug.
  const root = await mkdtemp(join(tmpdir(), 'demi-bash-utf8-'))
  const text = '# 林夏·主角\n顾深 — 反派 🎬\n'
  await writeFile(join(root, 'note.md'), text)
  const env = new BashEnvironment({
    host: new LocalHost(root),
    initialEnv: { PATH: process.env.PATH ?? '' },
  })

  const cat = await env.exec({ script: 'cat note.md' })
  expect(cat.stdout.delta).toBe(text)

  const head = await env.exec({ shellId: cat.shellId, script: 'head -1 note.md' })
  expect(head.stdout.delta).toBe('# 林夏·主角\n')

  // sed already emits decoded text — it must keep working unchanged.
  const sed = await env.exec({ shellId: cat.shellId, script: 'sed -n 2p note.md' })
  expect(sed.stdout.delta).toBe('顾深 — 反派 🎬\n')
})
