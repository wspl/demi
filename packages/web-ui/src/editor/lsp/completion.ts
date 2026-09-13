import type { AppContext } from 'vue'
import { autocompletion, type CompletionSource, type Completion } from '@codemirror/autocomplete'
import { type Extension } from '@codemirror/state'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { findPluginForResource, toLspPosition, fromLspPosition, type LspRange } from './utils'
import { VueRenderer } from '../render/vueRenderer'
import CompletionInfoVue from '../components/editor/CompletionInfo.vue'

interface LspCompletionItem {
  label: string
  labelDetails?: { detail?: string; description?: string }
  kind?: number
  detail?: string
  insertText?: string
  insertTextFormat?: number
  sortText?: string
  filterText?: string
  textEdit?: { range: LspRange; newText: string }
  documentation?: string | { kind: string; value: string }
  data?: unknown
}

function getCompletionDocumentation(item: LspCompletionItem | null | undefined): string | undefined {
  if (!item) return undefined
  return typeof item.documentation === 'string'
    ? item.documentation
    : item.documentation?.value
}

const kindToType: Record<number, string> = {
  1: 'text', 2: 'method', 3: 'function', 4: 'class', 5: 'property',
  6: 'variable', 7: 'class', 8: 'interface', 9: 'namespace', 10: 'property',
  12: 'constant', 13: 'constant', 14: 'keyword', 20: 'constant', 21: 'constant',
  22: 'class', 25: 'type',
}

function createLspCompletionSource(host: EditorHost, appContext: AppContext | null): CompletionSource {
  return async (context) => {
    const resourceUri = context.state.facet(lspFileUri)
    const target = findPluginForResource(host, resourceUri)
    if (!target?.capabilities.completionProvider) return null

    const triggerChar = context.state.sliceDoc(context.pos - 1, context.pos)
    const triggers = target.capabilities.completionProvider.triggerCharacters ?? []
    const isIdentifierChar = /[\p{L}\p{N}_$]/u.test(triggerChar)
    if (!context.explicit && !isIdentifierChar && !triggers.includes(triggerChar)) return null

    const result = await host.lsp.sendRequest(target, 'textDocument/completion', {
      textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
      position: toLspPosition(context.state.doc, context.pos),
      context: {
        triggerCharacter: triggerChar,
        triggerKind: context.explicit ? 1 : 2,
      },
    }) as { items?: LspCompletionItem[] } | LspCompletionItem[] | null
    if (!result) return null

    const items = Array.isArray(result) ? result : (result.items ?? [])
    if (items.length === 0) return null

    const word = context.state.wordAt(context.pos)
    const hasResolve = !!target.capabilities.completionProvider.resolveProvider

    return {
      from: word?.from ?? context.pos,
      options: items.map((item): Completion => {
        let text = item.insertText ?? item.label
        let from: number | undefined
        if (item.textEdit && 'range' in item.textEdit) {
          text = item.textEdit.newText
          from = fromLspPosition(context.state.doc, item.textEdit.range.start)
        }
        const completion: Completion = {
          label: item.label,
          apply: text !== item.label ? text : undefined,
          type: item.kind ? kindToType[item.kind] : undefined,
          detail: item.labelDetails?.description || item.labelDetails?.detail || item.detail || undefined,
          sortText: item.sortText,
          ...(from !== undefined ? { from } : {}),
        }
        if (hasResolve) {
          const lspItem = item
          completion.info = () => {
            const container = document.createElement('div')
            let renderer: VueRenderer | null = null
            let destroyed = false

            const renderInfo = (detail?: string, documentation?: string) => {
              if (destroyed) return false
              if (!detail && !documentation) return false
              renderer?.destroy()
              renderer = new VueRenderer(CompletionInfoVue, {
                detail,
                documentation,
                renderMarkdown: (value: string) => host.markdown.render(value),
              }, { appContext })
              container.replaceChildren(renderer.dom)
              return true
            }

            renderInfo(item.detail, getCompletionDocumentation(item))

            void host.lsp.sendRequest(target, 'completionItem/resolve', lspItem).then((resolved) => {
              const completionResult = resolved as LspCompletionItem | null
              if (!completionResult) return
              renderInfo(completionResult.detail, getCompletionDocumentation(completionResult))
            })

            return {
              dom: container,
              destroy: () => {
                destroyed = true
                renderer?.destroy()
              },
            }
          }
        }
        return completion
      }),
    }
  }
}

export function lspCompletionExtension(host: EditorHost, appContext: AppContext | null): Extension {
  return autocompletion({ override: [createLspCompletionSource(host, appContext)] })
}
