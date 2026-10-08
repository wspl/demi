import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useTryAgain } from '../useTryAgain'

// Try Again shows only the attempt the user asked for
// (`direct-channel.md` § What the user sees). The host here starts an
// attempt synchronously, as the product's and the gallery's do. Pure; no
// timers.

function host(starts = true) {
  const trying = ref(false)
  let started = 0
  const scope = effectScope()
  const button = scope.run(() =>
    useTryAgain(
      () => trying.value,
      () => {
        if (!starts)
          return
        started += 1
        trying.value = true
      },
    ),
  )!
  return { trying, button, started: () => started, scope }
}

test('an attempt Demi starts by itself shows no loading', () => {
  const { trying, button, scope } = host()
  trying.value = true
  expect(button.loading.value).toBe(false)
  scope.stop()
})

test('a click while Demi’s attempt runs starts none and shows that one until it ends', async () => {
  const { trying, button, started, scope } = host()
  trying.value = true
  await button.click()
  expect(started()).toBe(0)
  expect(button.loading.value).toBe(true)
  trying.value = false
  expect(button.loading.value).toBe(false)
  // Demi's next attempt is its own again.
  trying.value = true
  expect(button.loading.value).toBe(false)
  scope.stop()
})

test('a click while nothing runs starts an attempt and shows it until it ends', async () => {
  const { trying, button, started, scope } = host()
  await button.click()
  expect(started()).toBe(1)
  expect(button.loading.value).toBe(true)
  trying.value = false
  expect(button.loading.value).toBe(false)
  scope.stop()
})

test('a click the host starts no attempt for leaves a later attempt of Demi’s unshown', async () => {
  const { trying, button, scope } = host(false)
  await button.click()
  expect(button.loading.value).toBe(false)
  trying.value = true
  await nextTick()
  expect(button.loading.value).toBe(false)
  scope.stop()
})
