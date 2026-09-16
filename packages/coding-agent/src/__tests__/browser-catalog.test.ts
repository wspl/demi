import { expect, test } from 'bun:test'
import { browserErrorCodeSchema, browserOperations } from '@demicodes/browser-protocol'
import { CommandRegistry, parseCommandInput, renderCommandHelp } from '@demicodes/shell'
import { createBrowserGroup } from '../commands/browser/group'

const tab = `t_${'a'.repeat(22)}`

test('browser catalog declares every family and one input source per operand', () => {
  const group = createBrowserGroup()
  const registry = new CommandRegistry()
  registry.register(group)
  for (const operation of Object.keys(browserOperations)) {
    const parsed = parseCommandInput(group, ['browser', ...operation.split('.'), '--help'])
    expect(parsed.help).toBe(true)
  }
  for (const [command, body, field] of [
    [['eval', tab], 'document.title', 'expression'],
    [['find', tab, '--query'], '{"match":{"role":"button"}}', 'body'],
    [['cdp', 'send', tab, 'Network.enable'], '{}', 'params'],
    [['webmcp', 'call', tab, 'search', '--tools', 'tools-1'], '{}', 'arguments'],
  ] satisfies [string[], string, string][]) {
    expect(parseCommandInput(group, ['browser', ...command], body).values[field]).toBe(body)
    expect(() => parseCommandInput(group, ['browser', ...command, `--${field}`, body])).toThrow()
  }
  expect(parseCommandInput(group, ['browser', 'type', tab, '--text', 'hello']).values.text).toBe('hello')
  expect(parseCommandInput(group, ['browser', 'key', tab, '--key', 'ControlOrMeta+A']).values.key).toBe('ControlOrMeta+A')
  expect(parseCommandInput(group, ['browser', 'content', 'fetch', '--url', 'https://example.test/']).values.url).toEqual(['https://example.test/'])
  expect(renderCommandHelp(group)).toContain('probe')
  expect(new Set(browserErrorCodeSchema.options).size).toBe(browserErrorCodeSchema.options.length)
})

test('browser results enforce the catalog fields and preserve nested inspect nodes', () => {
  expect(browserOperations.inspect.result.parse({
    tab, url: 'about:blank', title: '', view: 'accessibility', truncated: false,
    tree: [{ role: 'main', children: [{ role: 'checkbox', states: ['checked=false'] }] }],
  }).tree[0]?.children?.[0]?.states).toEqual(['checked=false'])
  expect(browserOperations['dialog.accept'].result.safeParse({ handled: true }).success).toBe(false)
  expect(browserOperations['dialog.accept'].result.parse({ type: 'confirm', outcome: 'accepted' }).outcome).toBe('accepted')
  expect(browserOperations['content.read'].result.safeParse({ url: '', title: '', format: 'html', path: '/tmp/page.html' }).success).toBe(true)
})
