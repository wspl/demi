import { describe, expect, test } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { LocalHost } from '@demicodes/host-virtual/testing'
import { type OutsideReason, parseTinybash } from '../index'

/**
 * The refusal table: each row of `tinybash.md` refused unquoted and, where a
 * quoted spelling exists, accepted quoted. Parse-only; nothing runs.
 */

const roots = new Map([['demi', (argv: readonly string[]) => (argv[0] === 'file' ? argv.slice(2) : [])]])
const namespace = ['/home/demi', '/tmp']
const state = { cwd: '/home/demi', home: '/home/demi', vars: { HOME: '/home/demi', X: 'grep' } }

async function outsideOf(script: string): Promise<OutsideReason | null> {
  const result = await parseTinybash(script, roots, namespace, state)
  return result.kind === 'outside' ? result.reason : null
}

async function expectOutside(script: string, kind: OutsideReason['kind'], found?: string) {
  const reason = await outsideOf(script)
  expect(reason, script).not.toBeNull()
  expect(reason!.kind, script).toBe(kind)
  if (found !== undefined) {
    const actual = 'found' in reason! ? reason.found : 'name' in reason! ? reason.name : 'flag' in reason! ? reason.flag : 'path' in reason! ? reason.path : 'pattern' in reason! ? reason.pattern : ''
    expect(actual, script).toBe(found)
  }
}

async function expectInside(script: string) {
  expect(await outsideOf(script), script).toBeNull()
}

describe('substitutions and parameters', () => {
  test('command and arithmetic substitution', async () => {
    await expectOutside('echo $(date)', 'grammar', '$(')
    await expectOutside('echo `date`', 'grammar', '`')
    await expectOutside('echo "$(date)"', 'grammar', '$(')
    await expectOutside('echo $((1+2))', 'grammar', '$((')
    await expectInside("echo '$(date)' '`date`'")
  })
  test('parameter operators and positionals', async () => {
    await expectOutside('echo ${X:-default}', 'grammar', '${X:-default}')
    await expectOutside('echo ${#X}', 'grammar')
    await expectOutside('echo $?', 'grammar', '$?')
    await expectOutside('echo $1', 'grammar', '$1')
    await expectOutside('echo $@', 'grammar', '$@')
    await expectOutside('echo "$*"', 'grammar', '$*')
    await expectInside('echo $X ${X} "$X" costs $ 5 "$"')
  })
  test('brace expansion and recursive globs', async () => {
    await expectOutside('echo {a,b}', 'grammar', '{a,b}')
    await expectOutside('echo a{1,2}', 'grammar', '{1,2}')
    await expectOutside('ls src/**/*.ts', 'grammar', '**')
    await expectInside("echo '{a,b}' \"{}\" {} {x}")
    await expectOutside('echo {1..3}', 'grammar', '{1..3}')
  })
})

describe('control flow, grouping, jobs', () => {
  test('reserved words', async () => {
    for (const script of ['if true; then echo a; fi', 'for x in a b; do echo $x; done', 'while true; do true; done', 'case x in a) ;; esac', 'function f { true; }', '! true', 'time ls', '[[ -f x ]]']) {
      expect((await outsideOf(script))?.kind, script).toBe('grammar')
    }
    await expectInside('echo if for while case function')
  })
  test('subshells and grouping', async () => {
    await expectOutside('(cd src; ls)', 'grammar', '(')
    await expectOutside('{ echo a; echo b; }', 'grammar', '{')
  })
  test('job control and process substitution', async () => {
    await expectOutside('sleep 5 &', 'grammar', '&')
    await expectOutside('diff <(ls a) <(ls b)', 'grammar', '<(')
    await expectOutside('echo a > >(cat)', 'grammar', '>(')
    await expectOutside('echo a ;; echo b', 'grammar', ';;')
    await expectOutside('cat file |& tee', 'grammar', '|&')
    await expectInside('echo "a & b" \'&\'')
  })
  test('redirection forms outside the grammar', async () => {
    await expectOutside('echo a 1> f', 'grammar', '1>')
    await expectOutside('echo a >&2', 'grammar', '>&')
    await expectOutside('echo a 2>&2', 'grammar', '2>&2')
    await expectOutside('echo a &>> f', 'grammar', '&>>')
    await expectInside('echo a > f 2> g 2>&1 &> h >> i < j')
  })
})

describe('programs, flags and patterns', () => {
  test('unknown programs and the refused list', async () => {
    for (const name of ['python3', 'git', 'awk', 'jq', 'xargs', 'perl', 'export', 'source', 'eval', 'exit', 'sleep', 'kill', 'date', 'which']) {
      await expectOutside(`${name} x`, 'program', name)
    }
    await expectOutside('ls | python3', 'program', 'python3')
  })
  test('a program from a variable is checked after expansion', async () => {
    await expectInside('$X beta notes.txt')
    await expectOutside('Y=awk; $Y x', 'program', 'awk')
  })
  test('flags outside the whitelist', async () => {
    await expectOutside('grep -P x f', 'flag', '-P')
    await expectOutside('grep --color x f', 'flag', '--color')
    await expectOutside('sed -i s/a/b/ f', 'flag', '-i')
    await expectOutside('sed -n s/a/b/p f', 'flag', 's/a/b/p')
    await expectOutside('sed 5p f', 'flag', 'without -n')
    await expectOutside('ls -t', 'flag', '-t')
    await expectOutside('head -n -5 f', 'flag', '-n -5')
    await expectOutside('tail -f log', 'flag', '-f')
    await expectOutside('find . -exec rm {} \\;', 'flag')
    await expectOutside('find . -newer f', 'flag', '-newer')
    await expectOutside('sort -t, -k2 f', 'flag', '-t')
    await expectOutside('wc -m f', 'flag', '-m')
    await expectOutside('cat -A f', 'flag', '-A')
    await expectOutside('echo a | tr -s " "', 'flag', '-s')
    await expectOutside('[ -x f ]', 'flag', '-x')
    await expectOutside('[ a -a b ]', 'flag')
    await expectOutside('cd -', 'flag', '-')
    await expectOutside('rm -i f', 'flag', '-i')
    await expectOutside('mkdir -m 700 d', 'flag', '-m')
    await expectOutside('cp -a x y', 'flag', '-a')
    await expectOutside('printf -v x a', 'flag', '-v')
    await expectOutside("printf '%b\\n' x", 'flag', '%b')
    await expectOutside("printf '%*d' 3 1", 'flag')
    await expectOutside("printf '%1$s' a", 'flag')
    await expectOutside("printf '%5.2f' 1", 'flag', '%5.2f')
    await expectOutside("printf '%05s' a", 'flag', '%05s')
    await expectOutside("printf '\\u00e9'", 'flag')
    await expectInside("printf '%5s|%-3d|%x|%%|%.2s\\n' a 1 255 abc; printf -- '-%s' x")
  })
  test('grep patterns without a translation', async () => {
    await expectOutside("grep '\\(a\\)\\1' f", 'pattern')
    await expectOutside("grep -E '(a)\\1' f", 'pattern')
    await expectOutside("grep 'a\\{2\\}' f", 'pattern')
    await expectOutside("grep '[[=a=]]' f", 'pattern')
    await expectInside("grep -E 'a{2}|b+' f; grep 'a\\|b' f; grep -F '(' f")
  })
})

describe('paths and the namespace', () => {
  test('absolute paths outside', async () => {
    await expectOutside('cat /etc/passwd', 'path', '/etc/passwd')
    await expectOutside('echo a > /var/log/x', 'path', '/var/log/x')
    await expectOutside('cd /', 'path', '/')
    await expectOutside('ls /usr/*', 'path', '/usr/*')
    await expectOutside('demi file read /etc/hosts', 'path', '/etc/hosts')
    await expectOutside('[ -f /etc/hosts ]', 'path', '/etc/hosts')
    await expectOutside('cat < /proc/cpuinfo', 'path', '/proc/cpuinfo')
    await expectOutside('cd ..', 'path', '..')
    await expectOutside('cat ../../etc/passwd', 'path', '../../etc/passwd')
  })
  test('inside the namespace', async () => {
    await expectInside('cat /home/demi/x /tmp/y; echo a > /dev/null; cat < /dev/null')
    await expectInside('cd /tmp; cat x; cd /home/demi; cat notes.txt ../demi/x')
    await expectInside('cat ~/x "$HOME/y" $HOME/z')
    await expectInside('demi file read notes.txt; demi todo add /anything/not/a/path')
  })
  test('a cd whose outcome is unknown keeps both states', async () => {
    // Without a filesystem `cd src` may fail, so `..` is checked from the home too.
    await expectOutside('cd src; cd ..', 'path', '..')
    await expectOutside('cd src; cat ../notes.txt', 'path', '../notes.txt')
    await expectInside('cd src; cat src/notes.txt notes.txt; cd lib; cat x')
    // The home and the namespace roots exist by contract.
    await expectInside('cd; cd src; cd; cat ../demi/x')
    await expectInside('cd /tmp; cat ../home/demi/x')
  })
  test('a cd decided against the filesystem', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'tinybash-outside-'))
    try {
      mkdirSync(join(dir, 'src/lib'), { recursive: true })
      const host = new LocalHost(dir, { storeRoot: join(dir, '.store') })
      const local = { cwd: dir, home: dir, vars: { HOME: dir } }
      const decided = async (script: string) => (await parseTinybash(script, roots, [dir], local, host.fs)).kind
      expect(await decided('cd src; cat ../notes.txt')).toBe('script')
      expect(await decided('cd src; cd ..; cd src/lib; cd ../..; cat notes.txt')).toBe('script')
      expect(await decided('cd missing; cat ../notes.txt')).toBe('outside')
      // Not decided: after a command that can create or remove a directory, in a conditional branch, or with a glob.
      expect(await decided('mkdir src; cd src; cat ../notes.txt')).toBe('outside')
      expect(await decided('cat x || cd src; cat ../notes.txt')).toBe('outside')
      expect(await decided('cd s*; cat ../notes.txt')).toBe('outside')
      expect(await decided('cd src && cat ../notes.txt')).toBe('script')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
  test('a glob keeps its depth', async () => {
    await expectOutside('cat */../../x', 'path', '*/../../x')
    await expectInside('cat */../x src/*/../*.ts')
  })
  test('path and grammar conditions in a later statement refuse the whole script', async () => {
    await expectOutside('echo first; cat /etc/passwd', 'path')
    await expectOutside('echo first\necho second\necho $(x)', 'grammar')
  })
})

describe('not a script', () => {
  test('unterminated quotes, heredocs, carriage returns', async () => {
    await expectOutside("echo 'abc", 'syntax')
    await expectOutside('echo "abc', 'syntax')
    await expectOutside('cat <<EOF\nabc\n', 'syntax')
    await expectOutside('echo a\r\necho b', 'syntax')
    await expectOutside('echo a |', 'syntax')
    await expectOutside('echo a &&', 'syntax')
    await expectOutside('| cat', 'syntax')
  })
})

describe('refusal messages', () => {
  test('name the line and the way out', async () => {
    const result = await parseTinybash('echo ok\ncat /etc/passwd', roots, namespace, state)
    expect(result.kind).toBe('outside')
    if (result.kind === 'outside') expect(result.message).toBe('tinybash: line 2: /etc/passwd: no such place here; a path under the home, or a machine')
    const sed = await parseTinybash("sed -i 's/a/b/' f", roots, namespace, state)
    if (sed.kind === 'outside') expect(sed.message).toContain('demi file edit')
  })
})
