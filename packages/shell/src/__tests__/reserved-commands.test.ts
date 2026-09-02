import { expect, test } from 'bun:test'
import { CommandRegistry, type Command } from '../command'
import { RESERVED_COMMAND_NAMES } from '../reserved-names'

// Names a root may never take: shell words, the builtins every engine
// provides, the tools a script expects on a machine.
const EXPECTED_RESERVED = [
  '.', 'awk', 'bash', 'break', 'bun', 'cargo', 'cat', 'cd', 'chmod', 'command',
  'continue', 'cp', 'cut', 'docker', 'du', 'echo', 'exit', 'export', 'file',
  'find', 'git', 'grep', 'head', 'jobs', 'jq', 'local', 'ls', 'mkdir', 'mv',
  'nl', 'node', 'npm', 'pnpm', 'popd', 'printf', 'pushd', 'python', 'read',
  'return', 'rg', 'rm', 'sed', 'set', 'sh', 'shift', 'sort', 'source', 'stat',
  'tail', 'tee', 'test', 'touch', 'tree', 'tr', 'uniq', 'unset', 'wait', 'wc',
  'xargs', 'yarn', 'yq',
]

test('the reserved set covers every shell word, builtin and system tool', () => {
  for (const name of EXPECTED_RESERVED) {
    expect(RESERVED_COMMAND_NAMES.has(name)).toBe(true)
  }
})

test('registry rejects reserved names and accepts distinct ones', () => {
  const registry = new CommandRegistry(RESERVED_COMMAND_NAMES)
  const leaf: Command = { name: 'run', summary: 'x', kind: 'rpc', run: () => ({ exitCode: 0 }) }
  expect(() => registry.register({ name: 'grep', summary: 'x', subcommands: [leaf] })).toThrow(/reserved/)
  expect(() => registry.register({ name: 'go', summary: 'x', subcommands: [leaf] })).toThrow(/reserved/)
  registry.register({ name: 'my_tool', summary: 'x', subcommands: [leaf] })
  expect(registry.get('my_tool')).not.toBeNull()
})
