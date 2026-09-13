import type { AppContext } from 'vue'
import type { EditorView, Panel, ViewUpdate } from '@codemirror/view'
import { search, SearchQuery, setSearchQuery, getSearchQuery, findNext, findPrevious, replaceNext, replaceAll, closeSearchPanel } from '@codemirror/search'
import type { Extension } from '@codemirror/state'
import { VueRenderer } from './render/vueRenderer'
import SearchPanelVue from './components/editor/SearchPanel.vue'
import type { EditorHost } from './host/types'

function getMatchInfo(view: EditorView): { count: number; current: number } {
  const state = view.state
  const query = getSearchQuery(state)
  if (!query.valid) return { count: 0, current: 0 }

  const iter = query.getCursor(state.doc)
  let count = 0
  let current = 0
  const sel = state.selection.main.from
  let result = iter.next()
  while (!result.done) {
    count++
    if (current === 0 && result.value.from >= sel) {
      current = count
    }
    result = iter.next()
  }
  if (current === 0 && count > 0) current = count
  return { count, current }
}

function createSearchPanel(host: EditorHost, appContext: AppContext | null, view: EditorView): Panel {
  const query = getSearchQuery(view.state)

  const renderer = new VueRenderer(SearchPanelVue, {
    search: query.search,
    replace: query.replace,
    caseSensitive: query.caseSensitive,
    regexp: query.regexp,
    matchCount: 0,
    currentMatch: 0,
    onSearchChange: (val: string) => dispatchQuery({ search: val }),
    onReplaceChange: (val: string) => dispatchQuery({ replace: val }),
    onToggleCaseSensitive: () => dispatchQuery({ caseSensitive: !currentState.caseSensitive }),
    onToggleRegexp: () => dispatchQuery({ regexp: !currentState.regexp }),
    onNext: () => findNext(view),
    onPrev: () => findPrevious(view),
    onReplaceOne: () => replaceNext(view),
    onReplaceAll: () => replaceAll(view),
    onClose: () => closeSearchPanel(view),
    t: (key: string) => host.i18n.t(key),
  }, {
    appContext,
    useContainer: true,
  })

  let currentState = {
    search: query.search,
    replace: query.replace,
    caseSensitive: query.caseSensitive,
    regexp: query.regexp,
  }

  function dispatchQuery(patch: Partial<typeof currentState>) {
    currentState = { ...currentState, ...patch }
    renderer.updateProps(currentState)
    const q = new SearchQuery(currentState)
    view.dispatch({ effects: setSearchQuery.of(q) })
    updateMatchCount()
  }

  function updateMatchCount() {
    const info = getMatchInfo(view)
    renderer.updateProps({ matchCount: info.count, currentMatch: info.current })
  }

  return {
    dom: renderer.dom,
    top: false,
    mount() {
      updateMatchCount()
      const input = renderer.dom.querySelector('[main-field] input') as HTMLInputElement | null
      input?.focus()
      input?.select()
    },
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet) {
        updateMatchCount()
      }
      for (const tr of update.transactions) {
        for (const effect of tr.effects) {
          if (effect.is(setSearchQuery)) {
            const q = effect.value as SearchQuery
            currentState = {
              search: q.search,
              replace: q.replace,
              caseSensitive: q.caseSensitive,
              regexp: q.regexp,
            }
            renderer.updateProps(currentState)
            updateMatchCount()
          }
        }
      }
    },
    destroy() {
      renderer.destroy()
    },
  }
}

export function searchExtension(host: EditorHost, appContext: AppContext | null): Extension {
  return search({ top: false, createPanel: (view) => createSearchPanel(host, appContext, view) })
}
