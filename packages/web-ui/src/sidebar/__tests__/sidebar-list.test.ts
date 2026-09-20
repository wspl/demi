import { expect, test } from 'bun:test'
import { ref, type Ref } from 'vue'
import type { ListEntry } from '../list-model'
import { useSidebarList } from '../useSidebarList'

function rows(...ids: string[]): ListEntry[] {
  return ids.map((id) => ({ kind: 'conversation', id, projectId: null }))
}

function listOf(entries: Ref<ListEntry[]>, openId: string) {
  const noop = () => {}
  const list = useSidebarList(entries, ref<string | null>(openId), {
    open: noop,
    toggleFold: noop,
    fold: noop,
    rename: noop,
    togglePin: noop,
  })
  list.selectOnly(openId)
  return list
}

test('a conversation open before the list arrives is selected once it is listed', () => {
  const entries = ref<ListEntry[]>([])
  const list = listOf(entries, 'b')
  // Loading lists nothing for a while, then every row.
  entries.value = []
  list.prune()
  expect(list.selectedIds.value).toEqual([])
  entries.value = rows('a', 'b', 'c')
  list.prune()
  expect(list.selectedIds.value).toEqual(['b'])
})

test('rows that leave the list leave a wider selection, which keeps the rest', () => {
  const entries = ref(rows('a', 'b', 'c'))
  const list = listOf(entries, 'a')
  list.selected.value = new Set(['b', 'c'])
  entries.value = rows('a', 'b')
  list.prune()
  expect(list.selectedIds.value).toEqual(['b'])
  // With all of it gone, the selection is the open conversation again.
  entries.value = rows('a')
  list.prune()
  expect(list.selectedIds.value).toEqual(['a'])
})
