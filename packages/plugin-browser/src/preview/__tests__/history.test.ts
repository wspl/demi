import { expect, test } from 'bun:test'
import { TabHistory } from '../tabs'

// A tab's Back and Forward (`preview.md` § What the user sees) count the
// pages of every origin the tab showed, from the moves its top documents
// report; a page's own Navigation API sees only its origin's. A few
// microseconds.

test('Back and Forward count the pages of every origin, as a browser’s do', () => {
  const history = new TabHistory()
  const state = () => [history.canGoBack, history.canGoForward]
  // The user opens the app; a link goes on to another origin.
  history.moved('app', 'push')
  expect(state()).toEqual([false, false])
  history.moved('docs', 'push')
  expect(state()).toEqual([true, false])
  // Back returns to the app's entry: the docs page is ahead of it.
  history.moved('app', 'traverse')
  expect(state()).toEqual([false, true])
  history.moved('docs', 'traverse')
  expect(state()).toEqual([true, false])
  // A page that replaces its entry, or reloads, keeps its place.
  history.moved('docs-replaced', 'replace')
  history.moved('docs-replaced', 'reload')
  expect(state()).toEqual([true, false])
  // A new page from an earlier entry drops the ones ahead of it.
  history.moved('app', 'traverse')
  history.moved('checkout', 'push')
  expect(state()).toEqual([true, false])
  history.moved('app', 'traverse')
  expect(state()).toEqual([false, true])
  history.moved('checkout', 'traverse')
  expect(state()).toEqual([true, false])
})

test('a first move of any kind, and a move to an entry the tab never met, are new pages', () => {
  const history = new TabHistory()
  // The first page the tab shows, which its boot page reached by replacing itself.
  history.moved('first', 'replace')
  expect([history.canGoBack, history.canGoForward]).toEqual([false, false])
  history.moved('unknown', 'traverse')
  expect([history.canGoBack, history.canGoForward]).toEqual([true, false])
})
