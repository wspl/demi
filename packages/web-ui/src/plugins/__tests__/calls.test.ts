import { expect, test } from 'bun:test'
import { effectScope } from 'vue'
import { pendingCalls } from '../calls'
import { PluginCallError } from '../page'

// Cost: plain promises and one effect scope; milliseconds.

/** A call the test answers when it wants. */
function deferred() {
  let answer!: () => void
  let refuse!: (error: unknown) => void
  const promise = new Promise<void>((resolve, reject) => {
    answer = resolve
    refuse = reject
  })
  return { promise, answer, refuse }
}

/** Pending calls whose reports the test collects. */
function calls() {
  const reported: [string, unknown, 'report' | 'defect'][] = []
  const scope = effectScope()
  const pending = scope.run(() =>
    pendingCalls({
      report: (message, error) => void reported.push([message, error, 'report']),
      defect: (message, error) => void reported.push([message, error, 'defect']),
    }),
  )!
  return { pending, reported, scope }
}

test("a control's call is pending until it is answered, success or failure, and a failure is reported", async () => {
  const { pending, reported } = calls()
  const first = deferred()
  const switching = pending.run('src_1', 'Could not switch the skill', () => first.promise)
  expect(pending.pending.value).toEqual(['src_1'])
  // A second click while the first waits does nothing.
  let called = false
  await pending.run('src_1', 'Could not switch the skill', async () => {
    called = true
  })
  expect(called).toBe(false)
  first.answer()
  await switching
  expect(pending.pending.value).toEqual([])
  expect(reported).toEqual([])

  const second = deferred()
  const refusal = new PluginCallError('skill_name_taken', 'A skill named "review" is on')
  const refused = pending.run('src_1', 'Could not switch the skill', () => second.promise)
  second.refuse(refusal)
  await refused
  expect(pending.pending.value).toEqual([])
  expect(reported).toEqual([['Could not switch the skill', refusal, 'report']])
})

test('a call the end of its scope aborts reports nothing', async () => {
  const { pending, reported, scope } = calls()
  const updating = pending.run('src_1', 'Could not update the source', (signal) =>
    new Promise((_resolve, reject) => signal.addEventListener('abort', () => reject(signal.reason))),
  )
  scope.stop()
  await updating
  expect(pending.pending.value).toEqual([])
  expect(reported).toEqual([])
})
