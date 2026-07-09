import { expect, test } from 'bun:test'
import { CommandRegistry } from '../command'
import { DEMI_PORTABLE_COMMANDS, RESERVED_COMMAND_NAMES } from '../portable-commands'

// The historical hand-maintained reservation list; the derived set must remain
// a superset so no previously-rejected name silently becomes registrable.
const LEGACY_RESERVED = [
  '.', 'awk', 'bash', 'break', 'bun', 'cargo', 'cat', 'cd', 'chmod', 'command',
  'continue', 'cp', 'cut', 'docker', 'du', 'echo', 'exit', 'export', 'file',
  'find', 'git', 'grep', 'head', 'jobs', 'jq', 'local', 'ls', 'mkdir', 'mv',
  'nl', 'node', 'npm', 'pnpm', 'popd', 'printf', 'pushd', 'python', 'read',
  'return', 'rg', 'rm', 'sed', 'set', 'sh', 'shift', 'sort', 'source', 'stat',
  'tail', 'tee', 'test', 'touch', 'tree', 'tr', 'uniq', 'unset', 'wait', 'wc',
  'xargs', 'yarn', 'yq',
]

test('derived reserved names cover every portable command and the legacy list', () => {
  for (const name of DEMI_PORTABLE_COMMANDS) {
    expect(RESERVED_COMMAND_NAMES.has(name)).toBe(true)
  }
  for (const name of LEGACY_RESERVED) {
    expect(RESERVED_COMMAND_NAMES.has(name)).toBe(true)
  }
})

test('registry rejects reserved names and accepts distinct ones', () => {
  const registry = new CommandRegistry()
  const leaf = { name: 'run', summary: 'x', examples: [] as string[], run: () => ({ exitCode: 0 }) }
  expect(() => registry.register({ name: 'grep', summary: 'x', subcommands: [leaf] })).toThrow(/reserved/)
  expect(() => registry.register({ name: 'go', summary: 'x', subcommands: [leaf] })).toThrow(/reserved/)
  registry.register({ name: 'my_tool', summary: 'x', subcommands: [leaf] })
  expect(registry.get('my_tool')).not.toBeNull()
})
