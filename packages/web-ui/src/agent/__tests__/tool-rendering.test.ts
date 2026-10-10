import { expect, test } from 'bun:test'
import { pendingCallTitle, shellStatusLooks, shellTitle, toolRenderKind, trimToolSummary } from '../tool-rendering'

// Cost: pure, a few milliseconds for the file.

test('a shell call is titled by its description, and by its script without one', () => {
  expect(shellTitle({ description: 'Run unit tests', script: 'bun test' })).toBe('Run unit tests')
  expect(shellTitle({ description: '  ', script: 'bun test' })).toBe('bun test')
  expect(trimToolSummary(' a\n  b ')).toBe('a b')
})

test('only shell has its own rendering; a call of a removed or unknown tool is a generic card', () => {
  expect(toolRenderKind('shell')).toBe('shell')
  // Calls a transcript kept from before these tools were removed.
  for (const removed of ['shell_exec', 'shell_status', 'yield', 'shell_write', 'shell_abort'])
    expect(toolRenderKind(removed)).toBe('generic')
  expect(toolRenderKind('unknown_tool')).toBe('generic')
})

test('a call being written is titled by its description once written, and by its tool until then', () => {
  expect(pendingCallTitle({ toolName: 'shell', description: 'Write the categorizer' })).toBe('Write the categorizer')
  expect(pendingCallTitle({ toolName: 'shell', description: null })).toBe('Preparing a command…')
  expect(pendingCallTitle({ toolName: 'read_file', description: null })).toBe('read_file')
})

test('a script that only runs demi shell status looks at the commands it names, each once', () => {
  expect(shellStatusLooks('demi shell status 17')).toEqual(['17'])
  expect(shellStatusLooks('demi shell status 17 18 --wait 30s')).toEqual(['17', '18'])
  expect(shellStatusLooks('demi shell status --interval 600000 17\ndemi shell status 17 && demi shell status 19')).toEqual(['17', '19'])
  expect(shellStatusLooks('# the suite\ndemi shell status 17 --resident')).toEqual(['17'])
  // Anything else the script does runs a command of its own.
  expect(shellStatusLooks('demi shell status 17 | tail -n 5')).toBeNull()
  expect(shellStatusLooks('demi shell status 17\nbun test')).toBeNull()
  expect(shellStatusLooks("demi shell input 17 <<'EOF'\ny\nEOF\ndemi shell status 17")).toBeNull()
  expect(shellStatusLooks('demi shell stop 17')).toBeNull()
  expect(shellStatusLooks('demi shell status')).toBeNull()
})
