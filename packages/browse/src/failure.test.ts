// Scripts run as the tool's server runs them, against a stand-in scope, and
// Playwright's messages as Playwright 1.63 wrote them; a few milliseconds.
import { afterEach, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describeFailure } from './failure'
import { Script, type ScriptScope } from './script'
import { Failure } from './tool'

const folders: string[] = []

afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true })
  }
})

/** The report of `source` run as a call, with a `demi` whose `fail` fails as a helper does, after an await. */
async function reportOf(source: string, file: string | null = null): Promise<string[]> {
  const folder = mkdtempSync(join(tmpdir(), 'browse-script-'))
  folders.push(folder)
  const script = new Script(source, folder)
  const demi = {
    fail: async () => {
      await Promise.resolve()
      throw new Failure('The backend answered 500 for the conversations')
    },
  }
  const scope: ScriptScope = { page: undefined, context: undefined, expect: undefined, cdp: undefined, demi, keep: {}, console }
  try {
    await script.run(scope)
  } catch (error) {
    const line = script.lineOf(error)
    return describeFailure(error, { line, text: line === null ? '' : script.text(line), file })
  }
  throw new Error('the script did not fail')
}

test('a failure names the script\'s own line, whether the script, a helper it awaited or its syntax failed', async () => {
  const own = await reportOf('const shown = JSON.parse(\'{}\')\n\nshown.dialog.close()')
  expect(own[0]).toBe('Failed at line 3: shown.dialog.close()')
  expect(own[1]).toStartWith('  TypeError: ')
  // A helper's failure is its message, never the tool's stack.
  expect(await reportOf('// first\n\nawait demi.fail()', 'check.ts')).toEqual([
    'Failed at check.ts:3: await demi.fail()',
    '  The backend answered 500 for the conversations',
  ])
  expect(await reportOf('const x = 1\nlet x = 2')).toEqual([
    'Failed at line 2: let x = 2',
    '  The script does not compile: "x" has already been declared',
  ])
  // A callback the script gave a helper is named by its own line.
  expect((await reportOf('await [1].map(async () => {\n  throw new Error("in the callback")\n})[0]'))[0]).toBe('Failed at line 2: throw new Error("in the callback")')
})

const STRICT = `click: Error: strict mode violation: getByRole('button') resolved to 2 elements:
    1) <button>A</button> aka getByRole('button', { name: 'A' })
    2) <span role="button" class="relative inline-flex w-full cursor-default items-center">…</span> aka getByRole('button', { name: 'Remove' }).nth(1)

Call log:
  - waiting for getByRole('button')
`

const COVERED = `click: Timeout 1500ms exceeded.
Call log:
  - waiting for getByRole('button', { name: 'A' })
    - locator resolved to <button>A</button>
  - attempting click action
    2 × waiting for element to be visible, enabled and stable
      - element is visible, enabled and stable
      - scrolling into view if needed
      - done scrolling
      - <div role="dialog" aria-label="Cover">x</div> intercepts pointer events
    - retrying click action
    - waiting 20ms
    2 × waiting for element to be visible, enabled and stable
      - element is visible, enabled and stable
      - scrolling into view if needed
      - done scrolling
      - <div role="dialog" aria-label="Cover">x</div> intercepts pointer events
    - retrying click action
      - waiting 500ms
`

const ASSERTION = `Error: expect(locator).toBeVisible() failed

Locator: getByRole('button', { name: 'C' })
Expected: visible
Timeout: 500ms
Error: element(s) not found

Call log:
  - Expect "toBeVisible" getByRole('button', { name: 'C' }) with timeout 500ms
  - waiting for getByRole('button', { name: 'C' })
`

test('Playwright\'s failure keeps every element a locator matched and what covers an element, not its retries', () => {
  const where = { line: 4, text: 'await page.getByRole(\'button\').click()', file: null }
  expect(describeFailure(new Error(STRICT), where)).toEqual([
    'Failed at line 4: await page.getByRole(\'button\').click()',
    '  click: strict mode violation: getByRole(\'button\') resolved to 2 elements:',
    '      1) <button> aka getByRole(\'button\', { name: \'A\' })',
    '      2) <span> aka getByRole(\'button\', { name: \'Remove\' }).nth(1)',
    '  waiting for getByRole(\'button\')',
  ])
  expect(describeFailure(new Error(COVERED), where).slice(1)).toEqual([
    '  click: Timeout 1500ms exceeded.',
    '  waiting for getByRole(\'button\', { name: \'A\' })',
    '  locator resolved to <button>A</button>',
    '  <div role="dialog" aria-label="Cover">x</div> intercepts pointer events',
  ])
  // An assertion's message comes coloured on a terminal; the report is plain.
  expect(describeFailure(new Error(`\u001b[31m${ASSERTION}\u001b[39m`), where).slice(1)).toEqual([
    '  Error: expect(locator).toBeVisible() failed',
    '  Locator: getByRole(\'button\', { name: \'C\' })',
    '  Expected: visible',
    '  Timeout: 500ms',
    '  Error: element(s) not found',
    '  waiting for getByRole(\'button\', { name: \'C\' })',
  ])
})
