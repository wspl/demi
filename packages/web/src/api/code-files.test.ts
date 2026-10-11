import { afterEach, expect, test } from 'bun:test'
import { lookupCodeFiles } from './code-files'

// A reply's inline code paths go to the Host in one question, asked once
// (`file-previews.md` § Files named in messages). Pure: the backend is a
// stubbed fetch; a few milliseconds.

const realFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = realFetch
})

/** Answers each question with the files among its paths, or refuses it with `status`, and keeps the paths each asked. */
function stubHost(files: readonly string[], status = 200): string[][] {
  const asked: string[][] = []
  globalThis.fetch = (async (_input: RequestInfo | URL, init?: RequestInit) => {
    const paths: string[] = JSON.parse(String(init?.body)).paths
    asked.push(paths)
    return status === 200
      ? Response.json({ files: paths.filter((path) => files.includes(path)) })
      : Response.json({ code: 'host_stopped', message: 'The Cloud is stopped.' }, { status })
  }) as typeof fetch
  return asked
}

/** Polls `lookup` until the Host's answer is in; the deadline only guards against a hang. */
async function answer(lookup: () => ReadonlySet<string> | null): Promise<string[]> {
  const deadline = Date.now() + 5000
  for (;;) {
    const files = lookup()
    if (files)
      return [...files]
    if (Date.now() > deadline)
      throw new Error('no answer')
    await Bun.sleep(1)
  }
}

test('a reply asks about its paths once, together, and keeps the answer', async () => {
  const asked = stubHost(['/work/src/app.ts'])
  const paths = ['/work/src/app.ts', '/work/README.md']
  expect(lookupCodeFiles('c-1', paths, 'online')).toBeNull()
  expect(await answer(() => lookupCodeFiles('c-1', paths, 'online'))).toEqual(['/work/src/app.ts'])
  // Every render before the answer and after it asked nothing more.
  expect(asked).toEqual([paths])
})

test('a question a stopped Cloud could not answer goes again once the Host runs', async () => {
  const refused = stubHost([], 409)
  const paths = ['/home/demi/notes.md']
  expect(lookupCodeFiles('c-2', paths, 'stopped')).toBeNull()
  expect(lookupCodeFiles('c-2', paths, 'stopped')).toBeNull()
  expect(refused).toHaveLength(1)
  const answered = stubHost(['/home/demi/notes.md'])
  expect(await answer(() => lookupCodeFiles('c-2', paths, 'running'))).toEqual(['/home/demi/notes.md'])
  expect(answered).toHaveLength(1)
})
