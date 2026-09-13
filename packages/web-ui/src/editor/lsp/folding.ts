import { StateEffect, StateField, type Extension } from '@codemirror/state'
import { ViewPlugin, type ViewUpdate } from '@codemirror/view'
import { foldService } from '@codemirror/language'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { findPluginForResource } from './utils'

const refreshFolding = StateEffect.define<void>()
const foldingVersion = StateField.define<number>({
  create: () => 0,
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(refreshFolding)) return value + 1
    }
    return value
  },
})

export function lspFoldingExtension(host: EditorHost): Extension {
  let cachedUri = ''
  let cachedRanges: Array<{ startLine: number; endLine: number }> = []
  let fetchPromise: Promise<void> | null = null
  let inFlightUri = ''
  let requestVersion = 0

  function fetchRanges(uri: string, notify?: () => void) {
    if (fetchPromise && inFlightUri === uri) return fetchPromise
    if (cachedUri !== uri) cachedRanges = []
    cachedUri = uri
    inFlightUri = uri
    const version = ++requestVersion
    fetchPromise = (async () => {
      const target = findPluginForResource(host, uri)
      if (!target?.capabilities.foldingRangeProvider) {
        if (version !== requestVersion || cachedUri !== uri) return
        cachedRanges = []
        notify?.()
        return
      }
      const result = await host.lsp.sendRequest(target, 'textDocument/foldingRange', {
        textDocument: { uri: host.lsp.toDocumentUri(target, uri) },
      }) as Array<{ startLine: number; endLine: number }> | null
      if (version !== requestVersion || cachedUri !== uri) return
      cachedRanges = result ?? []
      notify?.()
    })().finally(() => {
      if (version === requestVersion) fetchPromise = null
    })
    return fetchPromise
  }

  const foldingRefreshPlugin = ViewPlugin.fromClass(class {
    private lastUri = ''

    constructor(view: ViewUpdate['view']) {
      this.refresh(view)
    }

    update(update: ViewUpdate) {
      if (update.docChanged || update.startState.facet(lspFileUri) !== update.state.facet(lspFileUri)) {
        this.refresh(update.view)
      }
    }

    private refresh(view: ViewUpdate['view']) {
      const uri = view.state.facet(lspFileUri)
      if (!uri || uri === this.lastUri) return
      this.lastUri = uri
      void fetchRanges(uri, () => {
        view.dispatch({ effects: refreshFolding.of() })
      })
    }
  })

  return [
    foldingVersion,
    foldingRefreshPlugin,
    foldService.of((state, lineStart, _lineEnd) => {
      state.field(foldingVersion)
      const uri = state.facet(lspFileUri)
      if (!uri) return null

      void fetchRanges(uri)

      const startLine = state.doc.lineAt(lineStart).number - 1
      const match = cachedRanges.find((r) => r.startLine === startLine)
      if (!match) return null

      const firstLine = state.doc.lineAt(lineStart)
      if (match.endLine <= startLine) return null
      const endLineNumber = Math.min(match.endLine + 1, state.doc.lines)
      const endLine = state.doc.line(endLineNumber)
      return { from: firstLine.to, to: endLine.to }
    }),
  ]
}
