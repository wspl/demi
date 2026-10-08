import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useRevealOpen } from '../reveal-open'

// Cost: reactivity alone, no DOM; milliseconds.

test('a project unfolds once the open conversation is listed in it, and stays folded when the user folds it again', async () => {
  const folded = ref(['p-demi', 'p-notes'])
  /** The project the open conversation is listed in, as the sidebar reads it from its rows. */
  const openProject = ref<string | null>(null)
  const scope = effectScope()
  scope.run(() => useRevealOpen({
    openProject: () => openProject.value,
    isFolded: (id) => folded.value.includes(id),
    unfold: (id) => {
      folded.value = folded.value.filter((each) => each !== id)
    },
  }))

  // A plain conversation unfolds nothing.
  openProject.value = null
  await nextTick()
  expect(folded.value).toEqual(['p-demi', 'p-notes'])

  // The project's new conversation is listed once its draft holds a character: its project unfolds.
  openProject.value = 'p-demi'
  await nextTick()
  expect(folded.value).toEqual(['p-notes'])

  // The user folds it again while the conversation stays open: it stays folded.
  folded.value = [...folded.value, 'p-demi']
  await nextTick()
  expect(folded.value).toEqual(['p-notes', 'p-demi'])
  scope.stop()
})
