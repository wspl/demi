import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { regionRetry } from '../region-retry'

/** A failed region's state, with its Retry, in an effect scope. */
function failedRegion() {
  const status = ref<'loading' | 'failed'>('failed')
  const detail = ref('HTTP 502')
  const scope = effectScope()
  const region = scope.run(() => regionRetry(() => [status.value, detail.value]))!
  return { status, detail, region, end: () => scope.stop() }
}

test('Retry shows loading at once, and a caller whose Retry fails the same way again shows the failure again', async () => {
  const { status, region, end } = failedRegion()
  let settle = () => {}
  region.retry(() => new Promise<void>((resolve) => (settle = resolve)))
  // Before the caller's work answered, and with its state still failed.
  expect(region.retrying.value).toBe(true)
  settle()
  await nextTick()
  expect(region.retrying.value).toBe(false)
  expect(status.value).toBe('failed')
  end()
})

test('a Retry that returns nothing loads until the caller’s state changes, and a refused one ends too', async () => {
  const { status, region, end } = failedRegion()
  region.retry(() => {
    status.value = 'loading'
  })
  expect(region.retrying.value).toBe(true)
  await nextTick()
  // The caller's own loading state shows now.
  expect(region.retrying.value).toBe(false)
  status.value = 'failed'
  region.retry(() => Promise.reject(new Error('refused')))
  expect(region.retrying.value).toBe(true)
  await Promise.resolve()
  await nextTick()
  expect(region.retrying.value).toBe(false)
  end()
})
