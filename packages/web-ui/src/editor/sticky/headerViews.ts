import { Compartment, EditorState, type Extension } from '@codemirror/state'
import { EditorView, GutterMarker, gutter } from '@codemirror/view'
import { foldGutter, language } from '@codemirror/language'
import { javascript } from '@codemirror/lang-javascript'
import { editorTheme, trackEditorView, untrackEditorView } from '../theme/cmTheme'

export interface StickyHeaderRenderItem {
  text: string
  lineNumber: number
}

interface StickyHeaderRow {
  host: HTMLDivElement
  view: EditorView
  lineNumberCompartment: Compartment
  languageCompartment: Compartment
  currentLanguageKey: string
  currentLineNumber: number
}

interface StickyHeaderGutterWidths {
  lineNumbers: number
  fold: number
}

class StickyLineNumberMarker extends GutterMarker {
  lineNumber: number
  constructor(lineNumber: number) {
    super()
    this.lineNumber = lineNumber
  }

  override eq(other: GutterMarker) {
    return other instanceof StickyLineNumberMarker && other.lineNumber === this.lineNumber
  }

  override toDOM() {
    const node = document.createElement('span')
    node.className = 'cm-lineNumber'
    node.textContent = String(this.lineNumber)
    return node
  }
}

function createStickyLineNumberGutter(lineNumber: number) {
  const marker = new StickyLineNumberMarker(lineNumber)
  return gutter({
    class: 'cm-lineNumbers',
    renderEmptyElements: true,
    lineMarker: () => marker,
    initialSpacer: () => marker,
  })
}

const stickyHeaderMiniTheme = EditorView.theme({
  '.cm-content': {
    padding: '0 !important',
  },
  '.cm-scroller': {
    overflow: 'hidden',
  },
  '.cm-line': {
    paddingTop: '0 !important',
    paddingBottom: '0 !important',
  },
})

export function createStickyHeaderViews(mainView: EditorView, host: HTMLElement) {
  const rows: StickyHeaderRow[] = []

  function getDefaultLanguageConfig(): { key: string; extensions: Extension[] } {
    const currentLanguage = mainView.state.facet(language)
    return currentLanguage
      ? { key: `main:${currentLanguage.name}`, extensions: [language.of(currentLanguage)] }
      : { key: 'main:none', extensions: [] }
  }

  function getVueScriptLanguage(lineNumber: number): 'js' | 'ts' | 'jsx' | 'tsx' | null {
    const lineIndex = Math.max(1, Math.min(lineNumber, mainView.state.doc.lines))
    const currentLine = mainView.state.doc.line(lineIndex).text

    if (/<\/?script\b/i.test(currentLine)) return null

    for (let index = lineIndex - 1; index >= 1; index -= 1) {
      const line = mainView.state.doc.line(index).text
      if (/<\/script>/i.test(line)) return null

      const scriptOpen = line.match(/<script\b([^>]*)>/i)
      if (!scriptOpen) continue

      const attrs = scriptOpen[1] ?? ''
      const lang = attrs.match(/\blang\s*=\s*["']([^"']+)["']/i)?.[1]?.toLowerCase()
      if (lang === 'ts') return 'ts'
      if (lang === 'tsx') return 'tsx'
      if (lang === 'jsx') return 'jsx'
      return 'js'
    }

    return null
  }

  function getLanguageConfig(lineNumber: number): { key: string; extensions: Extension[] } {
    const currentLanguage = mainView.state.facet(language)
    if (currentLanguage?.name === 'vue') {
      const vueScriptLanguage = getVueScriptLanguage(lineNumber)
      if (vueScriptLanguage) {
        return {
          key: `vue-script:${vueScriptLanguage}`,
          extensions: [javascript({
            typescript: vueScriptLanguage === 'ts' || vueScriptLanguage === 'tsx',
            jsx: vueScriptLanguage === 'jsx' || vueScriptLanguage === 'tsx',
          })],
        }
      }
    }

    return getDefaultLanguageConfig()
  }

  function createRow(item: StickyHeaderRenderItem) {
    const rowHost = document.createElement('div')
    rowHost.className = 'cm-stickyHeaderRow'
    rowHost.style.display = 'none'
    rowHost.style.contain = 'layout paint style'
    host.append(rowHost)

    const lineNumberCompartment = new Compartment()
    const languageCompartment = new Compartment()
    const languageConfig = getLanguageConfig(item.lineNumber)
    const view = new EditorView({
      parent: rowHost,
      state: EditorState.create({
        doc: item.text,
        extensions: [
          lineNumberCompartment.of(createStickyLineNumberGutter(item.lineNumber)),
          languageCompartment.of(languageConfig.extensions),
          foldGutter(),
          EditorState.readOnly.of(true),
          EditorView.editable.of(false),
          editorTheme(),
          stickyHeaderMiniTheme,
        ],
      }),
    })

    trackEditorView(view)

    return {
      host: rowHost,
      view,
      lineNumberCompartment,
      languageCompartment,
      currentLanguageKey: languageConfig.key,
      currentLineNumber: item.lineNumber,
    }
  }

  function ensureRowPool(minSize: number) {
    while (rows.length < minSize) {
      rows.push(createRow({ text: '', lineNumber: 1 }))
    }
  }

  function destroyRow(row: StickyHeaderRow) {
    untrackEditorView(row.view)
    row.view.destroy()
    row.host.remove()
  }

  return {
    render(
      items: StickyHeaderRenderItem[],
      gutterWidths: StickyHeaderGutterWidths = { lineNumbers: 0, fold: 0 },
    ) {
      ensureRowPool(items.length)

      items.forEach((item, index) => {
        let row = rows[index]
        if (!row) {
          row = createRow(item)
          rows.push(row)
        }
        row.host.style.display = ''

        if (row.view.state.doc.toString() !== item.text) {
          row.view.dispatch({
            changes: {
              from: 0,
              to: row.view.state.doc.length,
              insert: item.text,
            },
          })
        }
        if (row.currentLineNumber !== item.lineNumber) {
          row.view.dispatch({
            effects: row.lineNumberCompartment.reconfigure(createStickyLineNumberGutter(item.lineNumber)),
          })
          row.currentLineNumber = item.lineNumber
        }
        const languageConfig = getLanguageConfig(item.lineNumber)
        if (row.currentLanguageKey !== languageConfig.key) {
          row.view.dispatch({
            effects: row.languageCompartment.reconfigure(languageConfig.extensions),
          })
          row.currentLanguageKey = languageConfig.key
        }

        const lineNumbersGutter = row.host.querySelector<HTMLElement>('.cm-lineNumbers')
        if (lineNumbersGutter) {
          lineNumbersGutter.style.width = `${gutterWidths.lineNumbers}px`
          lineNumbersGutter.style.minWidth = `${gutterWidths.lineNumbers}px`
        }

        const foldGutterNode = row.host.querySelector<HTMLElement>('.cm-foldGutter')
        if (foldGutterNode) {
          foldGutterNode.style.width = `${gutterWidths.fold}px`
          foldGutterNode.style.minWidth = `${gutterWidths.fold}px`
        }
      })

      rows.slice(items.length).forEach((row) => {
        row.host.style.display = 'none'
      })
    },

    destroy() {
      while (rows.length > 0) {
        const row = rows.pop()
        if (row) destroyRow(row)
      }
    },
  }
}
