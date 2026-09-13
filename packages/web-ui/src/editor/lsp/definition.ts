import { keymap, EditorView, ViewPlugin, Decoration, type DecorationSet } from '@codemirror/view'
import { type Extension, StateField, StateEffect, RangeSetBuilder } from '@codemirror/state'
import { syntaxTree } from '@codemirror/language'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { findPluginForResource, toLspPosition, type LspLocation, type LspLocationLink } from './utils'

function jumpToDefinition(host: EditorHost, view: EditorView, pos: number) {
  const resourceUri = view.state.facet(lspFileUri)
  const target = findPluginForResource(host, resourceUri)
  if (!target?.capabilities.definitionProvider) return false

  host.lsp.sendRequest(target, 'textDocument/definition', {
    textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
    position: toLspPosition(view.state.doc, pos),
  }).then((result) => {
    const definitions = result as Array<LspLocation | LspLocationLink> | LspLocation | LspLocationLink | null
    if (!definitions) return
    const defs = Array.isArray(definitions) ? definitions : [definitions]
    if (defs.length === 0) return
    const def = defs[0]!
    const targetUri = 'targetUri' in def ? def.targetUri : def.uri
    const range = 'targetSelectionRange' in def ? def.targetSelectionRange : def.range
    const nextResourceUri = host.lsp.fromDocumentUri(target, targetUri, resourceUri)
    if (!nextResourceUri) return

    void host.navigation?.openResource(nextResourceUri, {
      selection: { start: range.start, end: range.end },
      reveal: true,
    })
  })
  return true
}

function getLinkRange(view: EditorView, pos: number): { from: number; to: number } | null {
  const tree = syntaxTree(view.state)
  const node = tree.resolveInner(pos)
  if (node.name === 'String' || node.name === 'TemplateString') {
    const from = node.from + 1
    const to = node.to - 1
    if (from < to && pos >= from && pos <= to) return { from, to }
  }
  return view.state.wordAt(pos)
}

const setLinkRange = StateEffect.define<{ from: number; to: number } | null>()

const linkDecoField = StateField.define<DecorationSet>({
  create() { return Decoration.none },
  update(deco, tr) {
    for (const effect of tr.effects) {
      if (effect.is(setLinkRange)) {
        if (!effect.value) return Decoration.none
        const { from, to } = effect.value
        const builder = new RangeSetBuilder<Decoration>()
        builder.add(from, to, Decoration.mark({ class: 'cm-definition-link' }))
        return builder.finish()
      }
    }
    return tr.docChanged ? Decoration.none : deco
  },
  provide: (f) => EditorView.decorations.from(f),
})

function createLinkPlugin(host: EditorHost) {
  return ViewPlugin.define((view) => {
    let isModDown = false
    let lastPos: number | null = null
    let pendingRequest = 0

    function clearLink() {
      if (view.state.field(linkDecoField) !== Decoration.none) {
        view.dispatch({ effects: setLinkRange.of(null) })
      }
    }

    function updateLink(pos: number | null) {
      pendingRequest++
      const requestId = pendingRequest

      if (!isModDown || pos === null) {
        clearLink()
        lastPos = null
        return
      }
      if (pos === lastPos) return
      lastPos = pos

      const range = getLinkRange(view, pos)
      if (!range) {
        clearLink()
        return
      }

      const resourceUri = view.state.facet(lspFileUri)
      const target = findPluginForResource(host, resourceUri)
      if (!target?.capabilities.definitionProvider) {
        clearLink()
        return
      }

      host.lsp.sendRequest(target, 'textDocument/definition', {
        textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
        position: toLspPosition(view.state.doc, pos),
      }).then((result) => {
        const definitions = result as Array<LspLocation | LspLocationLink> | LspLocation | LspLocationLink | null
        if (requestId !== pendingRequest) return
        if (!isModDown) return
        const defs = Array.isArray(definitions) ? definitions : definitions ? [definitions] : []
        if (defs.length > 0) {
          view.dispatch({ effects: setLinkRange.of(range) })
        } else {
          clearLink()
        }
      })
    }

    function onKeyDown(e: KeyboardEvent) {
      if (e.key === 'Meta' || e.key === 'Control') {
        isModDown = true
        updateLink(lastPos)
      }
    }

    function onKeyUp(e: KeyboardEvent) {
      if (e.key === 'Meta' || e.key === 'Control') {
        isModDown = false
        updateLink(null)
      }
    }

    function onMouseMove(e: MouseEvent) {
      const pos = view.posAtCoords({ x: e.clientX, y: e.clientY })
      isModDown = e.metaKey || e.ctrlKey
      updateLink(pos)
    }

    function onMouseLeave() {
      isModDown = false
      updateLink(null)
    }

    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('keyup', onKeyUp)
    view.dom.addEventListener('mousemove', onMouseMove)
    view.dom.addEventListener('mouseleave', onMouseLeave)

    return {
      destroy() {
        document.removeEventListener('keydown', onKeyDown)
        document.removeEventListener('keyup', onKeyUp)
        view.dom.removeEventListener('mousemove', onMouseMove)
        view.dom.removeEventListener('mouseleave', onMouseLeave)
      },
    }
  })
}

export function lspDefinitionExtension(host: EditorHost): Extension {
  return [
    linkDecoField,
    createLinkPlugin(host),
    EditorView.baseTheme({
      '.cm-definition-link': {
        textDecoration: 'underline',
        cursor: 'pointer',
      },
      '.cm-definition-link span': {
        color: 'var(--color-on-accent) !important',
      },
    }),
    keymap.of([{
      key: 'F12',
      run(view) {
        return jumpToDefinition(host, view, view.state.selection.main.head)
      },
      preventDefault: true,
    }]),
    EditorView.domEventHandlers({
      mousedown(event, view) {
        if (!(event.metaKey || event.ctrlKey)) return false
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY })
        if (pos === null) return false
        event.preventDefault()
        jumpToDefinition(host, view, pos)
        return true
      },
    }),
  ]
}
