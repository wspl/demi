import { h, render, type AppContext } from 'vue'
import { EditorSelection, type EditorState, type Extension } from '@codemirror/state'
import { keymap, runScopeHandlers, type EditorView, type Panel, type ViewUpdate } from '@codemirror/view'
import {
  closeSearchPanel,
  findNext,
  findPrevious,
  getSearchQuery,
  gotoLine,
  search,
  searchKeymap,
  SearchQuery,
  setSearchQuery,
} from '@codemirror/search'
import SearchPanel from './components/SearchPanel.vue'
import { searchMatches } from './searchMatches'

/** What the find bar edits: the text sought and how it matches. */
export interface FindOptions {
  search: string
  caseSensitive: boolean
  wholeWord: boolean
  regexp: boolean
}

function findOptions(query: SearchQuery): FindOptions {
  return {
    search: query.search,
    caseSensitive: query.caseSensitive,
    wholeWord: query.wholeWord,
    regexp: query.regexp,
  }
}

/** The first match at or after `from`, wrapping to the start of the text. */
function firstMatchFrom(state: EditorState, query: SearchQuery, from: number): { from: number; to: number } | null {
  const after = query.getCursor(state, from).next()
  if (!after.done)
    return after.value
  const first = query.getCursor(state).next()
  return first.done ? null : first.value
}

/** A changed query selects its first match from the selection on, as a browser's find does. */
function searchFor(view: EditorView, options: FindOptions): void {
  const query = new SearchQuery(options)
  if (query.eq(getSearchQuery(view.state)))
    return
  const match = query.valid ? firstMatchFrom(view.state, query, view.state.selection.main.from) : null
  view.dispatch({
    effects: setSearchQuery.of(query),
    ...(match ? { selection: EditorSelection.single(match.from, match.to), scrollIntoView: true } : {}),
    userEvent: 'select.search',
  })
}

function createSearchPanel(view: EditorView, appContext: AppContext | null): Panel {
  const dom = document.createElement('div')

  function draw(): void {
    const { ranges, complete } = searchMatches(view.state)
    const { from, to } = view.state.selection.main
    const panel = h(SearchPanel, {
      options: findOptions(getSearchQuery(view.state)),
      current: ranges.findIndex((range) => range.from === from && range.to === to) + 1,
      count: ranges.length,
      complete,
      onChange: (options: FindOptions) => searchFor(view, options),
      onPrevious: () => findPrevious(view),
      onNext: () => findNext(view),
      onClose: () => closeSearchPanel(view),
    })
    panel.appContext = appContext
    render(panel, dom)
  }

  // The search keys work in the bar as in the text: Escape closes it, Mod-f
  // selects the query, F3 and Mod-g step. Enter in the field steps too.
  dom.addEventListener('keydown', (event) => {
    if (runScopeHandlers(view, event, 'search-panel')) {
      event.preventDefault()
      return
    }
    if (event.key !== 'Enter' || !(event.target instanceof HTMLInputElement))
      return
    event.preventDefault()
    if (event.shiftKey)
      findPrevious(view)
    else
      findNext(view)
  })

  draw()
  return {
    dom,
    mount() {
      // CodeMirror's contract: the panel's field is the one tagged `main-field`.
      const field = dom.querySelector<HTMLInputElement>('input[main-field]')
      if (!field)
        return
      field.focus()
      field.select()
    },
    update(update: ViewUpdate) {
      const queried = update.transactions.some((transaction) =>
        transaction.effects.some((effect) => effect.is(setSearchQuery)))
      if (queried || update.selectionSet || update.docChanged)
        draw()
    },
    destroy() {
      render(null, dom)
    },
  }
}

// Go to line would open CodeMirror's own unstyled dialog.
const findKeys = searchKeymap.filter((binding) => binding.run !== gotoLine)

/** Find in the text: Mod-f opens the find bar below it. */
export function searchExtension(appContext: AppContext | null): Extension {
  return [
    search({ createPanel: (view) => createSearchPanel(view, appContext) }),
    keymap.of(findKeys),
  ]
}
