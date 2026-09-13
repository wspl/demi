import { StateField, StateEffect, type Extension, RangeSetBuilder } from '@codemirror/state'
import { EditorView, Decoration, type DecorationSet, ViewPlugin, type ViewUpdate } from '@codemirror/view'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { didRequestLspRefresh } from './refresh'
import { findPluginForResource } from './utils'

const TOKEN_CLASSES: Record<string, string> = {
  namespace: 'cm-sem-type',
  type: 'cm-sem-type',
  class: 'cm-sem-type',
  enum: 'cm-sem-type',
  interface: 'cm-sem-type',
  struct: 'cm-sem-type',
  typeParameter: 'cm-sem-type',
  parameter: 'cm-sem-parameter',
  variable: 'cm-sem-variable',
  property: 'cm-sem-property',
  enumMember: 'cm-sem-constant',
  function: 'cm-sem-function',
  method: 'cm-sem-function',
  macro: 'cm-sem-function',
  decorator: 'cm-sem-function',
}

const setTokens = StateEffect.define<DecorationSet>()

const tokenField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) {
      if (e.is(setTokens)) return e.value
    }
    return tr.docChanged ? Decoration.none : deco
  },
  provide: (f) => EditorView.decorations.from(f),
})

function buildDecorations(
  doc: { line(n: number): { from: number } },
  data: number[],
  legend: { tokenTypes: string[] },
): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>()
  let line = 0
  let col = 0
  const marks: Array<{ from: number; to: number; deco: Decoration }> = []

  for (let i = 0; i < data.length; i += 5) {
    const dLine = data[i]!
    const dCol = data[i + 1]!
    const len = data[i + 2]!
    const typeIdx = data[i + 3]!

    if (dLine > 0) { line += dLine; col = 0 }
    col += dCol

    const typeName = legend.tokenTypes[typeIdx]
    if (!typeName) continue
    const cls = TOKEN_CLASSES[typeName]
    if (!cls) continue

    const lineObj = doc.line(line + 1)
    const from = lineObj.from + col
    marks.push({ from, to: from + len, deco: Decoration.mark({ class: cls }) })
  }

  marks.sort((a, b) => a.from - b.from || a.to - b.to)
  for (const m of marks) builder.add(m.from, m.to, m.deco)
  return builder.finish()
}

function createFetchPlugin(host: EditorHost) {
  return ViewPlugin.fromClass(class {
    private timeout = -1
    private lastVersion = -1

    constructor(view: EditorView) {
      this.scheduleFetch(view)
    }

    update(update: ViewUpdate) {
      if (!update.docChanged && this.lastVersion >= 0 && !didRequestLspRefresh(update)) return
      this.scheduleFetch(update.view)
    }

    private scheduleFetch(view: EditorView) {
      if (this.timeout > -1) clearTimeout(this.timeout)
      this.timeout = window.setTimeout(() => {
        this.timeout = -1
        this.fetch(view)
      }, 500)
    }

    fetch(view: EditorView) {
      const resourceUri = view.state.facet(lspFileUri)
      const target = findPluginForResource(host, resourceUri)
      if (!target?.capabilities.semanticTokensProvider?.full) return

      const legend = target.capabilities.semanticTokensProvider.legend
      this.lastVersion = 0

      host.lsp.sendRequest(target, 'textDocument/semanticTokens/full', {
        textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
      }).then((result) => {
        const semanticTokens = result as { data: number[] } | null
        if (!semanticTokens?.data?.length) return
        const decos = buildDecorations(view.state.doc, semanticTokens.data, legend)
        view.dispatch({ effects: setTokens.of(decos) })
      }).catch(() => { /* ignore */ })
    }

    destroy() {
      if (this.timeout > -1) clearTimeout(this.timeout)
    }
  })
}

const semanticTheme = EditorView.baseTheme({
  '.cm-sem-variable': { color: 'var(--code-variable)' },
  '.cm-sem-constant': { color: 'var(--code-constant)' },
  '.cm-sem-parameter': { color: 'var(--code-variable)' },
  '.cm-sem-property': { color: 'var(--code-variable)' },
  '.cm-sem-function': { color: 'var(--code-function)' },
  '.cm-sem-type': { color: 'var(--code-type)' },
})

export function lspSemanticTokensExtension(host: EditorHost): Extension {
  return [tokenField, createFetchPlugin(host), semanticTheme]
}
