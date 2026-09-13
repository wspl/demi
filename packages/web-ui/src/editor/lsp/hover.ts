import type { AppContext } from 'vue'
import { hoverTooltip, type Tooltip } from '@codemirror/view'
import { type Extension } from '@codemirror/state'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { findPluginForResource, toLspPosition, fromLspRange, type LspRange } from './utils'
import { VueRenderer } from '../render/vueRenderer'
import HoverTooltipVue from '../components/editor/HoverTooltip.vue'

export function lspHoverExtension(host: EditorHost, appContext: AppContext | null): Extension {
  return hoverTooltip(async (view, pos): Promise<Tooltip | null> => {
    const resourceUri = view.state.facet(lspFileUri)
    const target = findPluginForResource(host, resourceUri)
    if (!target?.capabilities.hoverProvider) return null

    const result = await host.lsp.sendRequest(target, 'textDocument/hover', {
      textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
      position: toLspPosition(view.state.doc, pos),
    }) as {
      contents: { kind: string; value: string } | string | Array<string | { language: string; value: string }>
      range?: LspRange
    } | null
    if (!result) return null

    let text: string
    if (Array.isArray(result.contents)) {
      text = result.contents.map((c) =>
        typeof c === 'string' ? c : `\`\`\`${c.language}\n${c.value}\n\`\`\``,
      ).join('\n\n')
    } else if (typeof result.contents === 'string') {
      text = result.contents
    } else {
      text = result.contents.value
    }
    if (!text) return null

    const range = result.range ? fromLspRange(view.state.doc, result.range) : { from: pos, to: pos }

    return {
      pos: range.from,
      end: range.to,
      above: true,
      create() {
        const renderer = new VueRenderer(HoverTooltipVue, {
          text,
          renderMarkdown: (value: string) => host.markdown.render(value),
        }, { appContext })
        return {
          dom: renderer.dom,
          destroy: () => renderer.destroy(),
        }
      },
    }
  }, { hideOn: (tr) => tr.docChanged })
}
