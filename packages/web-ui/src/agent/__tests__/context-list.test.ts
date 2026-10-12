import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { expect, test } from 'bun:test'
import { effectScope, nextTick } from 'vue'

// The context card's header opens and closes its list of rows, and the
// browser keeps the choice for the next card (`compaction.md` § What fills
// the context). A page of happy-dom gives the browser's storage; a few
// milliseconds after the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
// After the page, since the storage helper looks for a browser as it loads.
const { useContextListOpen } = await import('../context-usage')

test('the list of rows is closed at first, and a card opened later shows it as the user last left it', async () => {
  const first = effectScope()
  const open = first.run(() => useContextListOpen())!
  expect(open.value).toBe(false)
  open.value = true
  await nextTick()
  first.stop()

  const later = effectScope()
  const reopened = later.run(() => useContextListOpen())!
  expect(reopened.value).toBe(true)
  reopened.value = false
  await nextTick()
  later.stop()

  const last = effectScope()
  expect(last.run(() => useContextListOpen())!.value).toBe(false)
  last.stop()
})
