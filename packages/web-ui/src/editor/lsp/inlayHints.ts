import { StateField, StateEffect, type Extension, RangeSetBuilder } from '@codemirror/state'
import { EditorView, Decoration, type DecorationSet, ViewPlugin, type ViewUpdate, WidgetType } from '@codemirror/view'
import type { EditorHost } from '../host/types'
import { lspFileUri } from './docSync'
import { didRequestLspRefresh } from './refresh'
import { findPluginForResource, toLspPosition, fromLspPosition } from './utils'

interface LspInlayHint {
  position: { line: number; character: number }
  label: string | Array<{ value: string }>
  kind?: number
  paddingLeft?: boolean
  paddingRight?: boolean
}

class InlayHintWidget extends WidgetType {
  text: string
  paddingLeft: boolean
  paddingRight: boolean

  constructor(text: string, paddingLeft: boolean, paddingRight: boolean) {
    super()
    this.text = text
    this.paddingLeft = paddingLeft
    this.paddingRight = paddingRight
  }

  override toDOM() {
    const span = document.createElement('span')
    span.className = 'cm-inlay-hint'
    span.textContent = this.text
    if (this.paddingLeft) span.style.marginLeft = '2px'
    if (this.paddingRight) span.style.marginRight = '2px'
    return span
  }

  override eq(other: InlayHintWidget) {
    return this.text === other.text && this.paddingLeft === other.paddingLeft && this.paddingRight === other.paddingRight
  }
}

const setHints = StateEffect.define<DecorationSet>()

const hintField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    for (const e of tr.effects) {
      if (e.is(setHints)) return e.value
    }
    return tr.docChanged ? Decoration.none : deco
  },
  provide: (f) => EditorView.decorations.from(f),
})

function createFetchPlugin(host: EditorHost) {
  return ViewPlugin.fromClass(class {
    private timeout = -1

    constructor(view: EditorView) {
      this.scheduleFetch(view)
    }

    update(update: ViewUpdate) {
      if (!update.docChanged && !update.viewportChanged && !didRequestLspRefresh(update)) return
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
      if (!target?.capabilities.inlayHintProvider) {
        if (view.state.field(hintField, false) !== Decoration.none) {
          queueMicrotask(() => {
            view.dispatch({ effects: setHints.of(Decoration.none) })
          })
        }
        return
      }

      const { from, to } = view.viewport
      host.lsp.sendRequest(target, 'textDocument/inlayHint', {
        textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
        range: {
          start: toLspPosition(view.state.doc, from),
          end: toLspPosition(view.state.doc, to),
        },
      }).then((result) => {
        const hints = result as LspInlayHint[] | null
        if (!hints?.length) {
          view.dispatch({ effects: setHints.of(Decoration.none) })
          return
        }

        const builder = new RangeSetBuilder<Decoration>()
        const sorted = hints
          .map((h) => ({
            pos: fromLspPosition(view.state.doc, h.position),
            label: typeof h.label === 'string' ? h.label : h.label.map((l) => l.value).join(''),
            paddingLeft: h.paddingLeft ?? false,
            paddingRight: h.paddingRight ?? false,
          }))
          .sort((a, b) => a.pos - b.pos)

        for (const h of sorted) {
          builder.add(h.pos, h.pos, Decoration.widget({
            widget: new InlayHintWidget(h.label, h.paddingLeft, h.paddingRight),
            side: 1,
          }))
        }
        view.dispatch({ effects: setHints.of(builder.finish()) })
      }).catch(() => { /* ignore */ })
    }

    destroy() {
      if (this.timeout > -1) clearTimeout(this.timeout)
    }
  })
}

const inlayTheme = EditorView.baseTheme({
  '.cm-inlay-hint': {
    color: 'var(--color-fg-subtle)',
    fontSize: '11px',
    fontFamily: 'inherit',
    border: '1px solid var(--color-line)',
    borderRadius: '3px',
    padding: '0 3px',
  },
})

export function lspInlayHintsExtension(host: EditorHost): Extension {
  return [hintField, createFetchPlugin(host), inlayTheme]
}
