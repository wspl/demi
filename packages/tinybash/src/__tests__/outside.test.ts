import { describe, expect, test } from 'bun:test'
import { type OutsideReason, parseTinybash } from '../index'

/**
 * The refusal table: each row of `tinybash.md` refused unquoted and, where a
 * quoted spelling exists, accepted quoted. Parse-only; nothing runs.
 */

const roots = new Map([['demi', (argv: readonly string[]) => (argv[0] === 'file' ? argv.slice(2) : [])]])
const namespace = ['/home/demi', '/tmp']
const state = { cwd: '/home/demi', home: '/home/demi', vars: { HOME: '/home/demi', X: 'grep' } }

function outsideOf(script: string): OutsideReason | null {
  const result = parseTinybash(script, roots, namespace, state)
  return result.kind === 'outside' ? result.reason : null
}

function expectOutside(script: string, kind: OutsideReason['kind'], found?: string) {
  const reason = outsideOf(script)
  expect(reason, script).not.toBeNull()
  expect(reason!.kind, script).toBe(kind)
  if (found !== undefined) {
    const actual = 'found' in reason! ? reason.found : 'name' in reason! ? reason.name : 'flag' in reason! ? reason.flag : 'path' in reason! ? reason.path : 'pattern' in reason! ? reason.pattern : ''
    expect(actual, script).toBe(found)
  }
}

function expectInside(script: string) {
  expect(outsideOf(script), script).toBeNull()
}

describe('substitutions and parameters', () => {
  test('command and arithmetic substitution', () => {
    expectOutside('echo $(date)', 'grammar', '$(')
    expectOutside('echo `date`', 'grammar', '`')
    expectOutside('echo "$(date)"', 'grammar', '$(')
    expectOutside('echo $((1+2))', 'grammar', '$((')
    expectInside("echo '$(date)' '`date`'")
  })
  test('parameter operators and positionals', () => {
    expectOutside('echo ${X:-default}', 'grammar', '${X:-default}')
    expectOutside('echo ${#X}', 'grammar')
    expectOutside('echo $?', 'grammar', '$?')
    expectOutside('echo $1', 'grammar', '$1')
    expectOutside('echo $@', 'grammar', '$@')
    expectOutside('echo "$*"', 'grammar', '$*')
    expectInside('echo $X ${X} "$X" costs $ 5 "$"')
  })
  test('brace expansion and recursive globs', () => {
    expectOutside('echo {a,b}', 'grammar', '{a,b}')
    expectOutside('echo a{1,2}', 'grammar', '{1,2}')
    expectOutside('ls src/**/*.ts', 'grammar', '**')
    expectInside("echo '{a,b}' \"{}\" {} {x}")
    expectOutside('echo {1..3}', 'grammar', '{1..3}')
  })
})

describe('control flow, grouping, jobs', () => {
  test('reserved words', () => {
    for (const script of ['if true; then echo a; fi', 'for x in a b; do echo $x; done', 'while true; do true; done', 'case x in a) ;; esac', 'function f { true; }', '! true', 'time ls', '[[ -f x ]]']) {
      expect(outsideOf(script)?.kind, script).toBe('grammar')
    }
    expectInside('echo if for while case function')
  })
  test('subshells and grouping', () => {
    expectOutside('(cd src; ls)', 'grammar', '(')
    expectOutside('{ echo a; echo b; }', 'grammar', '{')
  })
  test('job control and process substitution', () => {
    expectOutside('sleep 5 &', 'grammar', '&')
    expectOutside('diff <(ls a) <(ls b)', 'grammar', '<(')
    expectOutside('echo a > >(cat)', 'grammar', '>(')
    expectOutside('echo a ;; echo b', 'grammar', ';;')
    expectOutside('cat file |& tee', 'grammar', '|&')
    expectInside('echo "a & b" \'&\'')
  })
  test('redirection forms outside the grammar', () => {
    expectOutside('echo a 1> f', 'grammar', '1>')
    expectOutside('echo a >&2', 'grammar', '>&')
    expectOutside('echo a 2>&2', 'grammar', '2>&2')
    expectOutside('echo a &>> f', 'grammar', '&>>')
    expectInside('echo a > f 2> g 2>&1 &> h >> i < j')
  })
})

describe('programs, flags and patterns', () => {
  test('unknown programs and the refused list', () => {
    for (const name of ['python3', 'git', 'awk', 'jq', 'xargs', 'perl', 'export', 'source', 'eval', 'exit', 'sleep', 'kill', 'date', 'which']) {
      expectOutside(`${name} x`, 'program', name)
    }
    expectOutside('ls | python3', 'program', 'python3')
  })
  test('a program from a variable is checked after expansion', () => {
    expectInside('$X beta notes.txt')
    expectOutside('Y=awk; $Y x', 'program', 'awk')
  })
  test('flags outside the whitelist', () => {
    expectOutside('grep -P x f', 'flag', '-P')
    expectOutside('grep --color x f', 'flag', '--color')
    expectOutside('sed -i s/a/b/ f', 'flag', '-i')
    expectOutside('sed -n s/a/b/p f', 'flag', 's/a/b/p')
    expectOutside('sed 5p f', 'flag', 'without -n')
    expectOutside('ls -t', 'flag', '-t')
    expectOutside('head -n -5 f', 'flag', '-n -5')
    expectOutside('tail -f log', 'flag', '-f')
    expectOutside('find . -exec rm {} \\;', 'flag')
    expectOutside('find . -newer f', 'flag', '-newer')
    expectOutside('sort -t, -k2 f', 'flag', '-t')
    expectOutside('wc -m f', 'flag', '-m')
    expectOutside('cat -A f', 'flag', '-A')
    expectOutside('echo a | tr -s " "', 'flag', '-s')
    expectOutside('[ -x f ]', 'flag', '-x')
    expectOutside('[ a -a b ]', 'flag')
    expectOutside('cd -', 'flag', '-')
    expectOutside('rm -i f', 'flag', '-i')
    expectOutside('mkdir -m 700 d', 'flag', '-m')
    expectOutside('cp -a x y', 'flag', '-a')
    expectOutside('printf -v x a', 'flag', '-v')
    expectOutside("printf '%b\\n' x", 'flag', '%b')
    expectOutside("printf '%*d' 3 1", 'flag')
    expectOutside("printf '%1$s' a", 'flag')
    expectOutside("printf '%5.2f' 1", 'flag', '%5.2f')
    expectOutside("printf '%05s' a", 'flag', '%05s')
    expectOutside("printf '\\u00e9'", 'flag')
    expectInside("printf '%5s|%-3d|%x|%%|%.2s\\n' a 1 255 abc; printf -- '-%s' x")
  })
  test('grep patterns without a translation', () => {
    expectOutside("grep '\\(a\\)\\1' f", 'pattern')
    expectOutside("grep -E '(a)\\1' f", 'pattern')
    expectOutside("grep 'a\\{2\\}' f", 'pattern')
    expectOutside("grep '[[=a=]]' f", 'pattern')
    expectInside("grep -E 'a{2}|b+' f; grep 'a\\|b' f; grep -F '(' f")
  })
})

describe('paths and the namespace', () => {
  test('absolute paths outside', () => {
    expectOutside('cat /etc/passwd', 'path', '/etc/passwd')
    expectOutside('echo a > /var/log/x', 'path', '/var/log/x')
    expectOutside('cd /', 'path', '/')
    expectOutside('ls /usr/*', 'path', '/usr/')
    expectOutside('demi file read /etc/hosts', 'path', '/etc/hosts')
    expectOutside('[ -f /etc/hosts ]', 'path', '/etc/hosts')
    expectOutside('cat < /proc/cpuinfo', 'path', '/proc/cpuinfo')
    expectOutside('cd ..', 'path', '..')
    expectOutside('cat ../../etc/passwd', 'path', '../../etc/passwd')
  })
  test('inside the namespace', () => {
    expectInside('cat /home/demi/x /tmp/y; echo a > /dev/null; cat < /dev/null')
    expectInside('cd /tmp; cat x; cd /home/demi/src; cat ../notes.txt')
    expectInside('cat ~/x "$HOME/y" $HOME/z')
    expectInside('demi file read notes.txt; demi todo add /anything/not/a/path')
  })
  test('cd is simulated in order', () => {
    expectOutside('cd src; cd ..; cd ..', 'path', '..')
    expectInside('cd src; cd ..; cd src/lib; cd ../..')
  })
  test('path and grammar conditions in a later statement refuse the whole script', () => {
    expectOutside('echo first; cat /etc/passwd', 'path')
    expectOutside('echo first\necho second\necho $(x)', 'grammar')
  })
})

describe('not a script', () => {
  test('unterminated quotes, heredocs, carriage returns', () => {
    expectOutside("echo 'abc", 'syntax')
    expectOutside('echo "abc', 'syntax')
    expectOutside('cat <<EOF\nabc\n', 'syntax')
    expectOutside('echo a\r\necho b', 'syntax')
    expectOutside('echo a |', 'syntax')
    expectOutside('echo a &&', 'syntax')
    expectOutside('| cat', 'syntax')
  })
})

describe('refusal messages', () => {
  test('name the line and the way out', () => {
    const result = parseTinybash('echo ok\ncat /etc/passwd', roots, namespace, state)
    expect(result.kind).toBe('outside')
    if (result.kind === 'outside') expect(result.message).toBe('tinybash: line 2: /etc/passwd: no such place here; a path under the home, or a machine')
    const sed = parseTinybash("sed -i 's/a/b/' f", roots, namespace, state)
    if (sed.kind === 'outside') expect(sed.message).toContain('demi file edit')
  })
})
