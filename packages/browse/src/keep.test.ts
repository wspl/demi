// `keep` between calls of one server and across a restart of the server;
// scripts run as the server runs them, a few milliseconds.
import { afterEach, expect, test } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { restoreKeep, saveKeep, type Keep } from './keep'
import { Script } from './script'

const folders: string[] = []

afterEach(() => {
  for (const folder of folders.splice(0)) {
    rmSync(folder, { recursive: true, force: true })
  }
})

function folder(): string {
  const made = mkdtempSync(join(tmpdir(), 'browse-keep-'))
  folders.push(made)
  return made
}

/** Runs `source` as a call with `keep`; answers what it returned. */
function call(source: string, keep: Keep, calls: string): Promise<unknown> {
  const script = new Script(source, calls)
  return script.run({ page: undefined, context: undefined, expect: undefined, cdp: undefined, demi: {}, keep, console }).finally(() => script.remove())
}

test('what a call keeps, a later call finds, also after the server restarts for changed code; what cannot be copied is named', async () => {
  const calls = folder()
  const keep: Keep = {}
  await call('keep.conversation = "c-1"\nkeep.seen = new Map([["Reload", 2]])\nkeep.button = () => "a locator"', keep, calls)
  expect(await call('return [keep.conversation, keep.seen.get("Reload")]', keep, calls)).toEqual(['c-1', 2])

  const left = join(folder(), 'keep.bin')
  const lost = saveKeep(left, keep)
  expect(lost).toEqual([expect.stringMatching(/^keep\.button: /)])
  const next = restoreKeep(left)
  expect(await call('return [keep.conversation, keep.seen.get("Reload"), "button" in keep]', next, calls)).toEqual(['c-1', 2, false])
  // The file is used once: a server after that starts with nothing kept from before the restart.
  expect(restoreKeep(left)).toEqual({})
})
