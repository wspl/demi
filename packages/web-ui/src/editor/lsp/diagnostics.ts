import type { AppContext } from 'vue'
import { StateField, StateEffect, type Extension, RangeSetBuilder } from '@codemirror/state'
import { EditorView, Decoration, type DecorationSet, ViewPlugin, type ViewUpdate, hoverTooltip, type Tooltip } from '@codemirror/view'
import { lspFileUri } from './docSync'
import { fromLspPosition } from './utils'
import { VueRenderer } from '../render/vueRenderer'
import DiagnosticTooltipVue from '../components/editor/DiagnosticTooltip.vue'
import type { EditorDiagnostic, EditorHost } from '../host/types'

type Severity = 'error' | 'warning' | 'info' | 'hint'

const severityMap: Record<number, Severity> = {
  1: 'error',
  2: 'warning',
  3: 'info',
  4: 'hint',
}

interface ResolvedDiagnostic {
  from: number
  to: number
  severity: Severity
  message: string
  source?: string
}

const setDiags = StateEffect.define<ResolvedDiagnostic[]>()

const diagField = StateField.define<{ items: ResolvedDiagnostic[]; decos: DecorationSet }>({
  create: () => ({ items: [], decos: Decoration.none }),
  update(state, tr) {
    for (const e of tr.effects) {
      if (e.is(setDiags)) {
        const sorted = e.value.slice().sort((a, b) => a.from - b.from)
        const builder = new RangeSetBuilder<Decoration>()
        for (const d of sorted) {
          if (d.from < d.to) {
            builder.add(d.from, d.to, Decoration.mark({ class: `cm-diag-${d.severity}` }))
          }
        }
        return { items: e.value, decos: builder.finish() }
      }
    }
    if (tr.docChanged) return { items: [], decos: Decoration.none }
    return state
  },
  provide: (f) => EditorView.decorations.from(f, (s) => s.decos),
})

function resolve(view: EditorView, items: EditorDiagnostic[]): ResolvedDiagnostic[] {
  return items.map((d) => ({
    from: fromLspPosition(view.state.doc, d.range.start),
    to: fromLspPosition(view.state.doc, d.range.end),
    severity: severityMap[d.severity ?? 3] ?? 'info',
    message: d.message,
    source: d.source ?? undefined,
  }))
}

function createSyncPlugin(host: EditorHost) {
  return ViewPlugin.fromClass(class {
    private prevJson = ''
    private pending = -1

    constructor(view: EditorView) {
      this.scheduleSync(view)
    }

    update(update: ViewUpdate) {
      this.scheduleSync(update.view)
    }

    private scheduleSync(view: EditorView) {
      if (this.pending > -1) return
      this.pending = requestAnimationFrame(() => {
        this.pending = -1
        const uri = view.state.facet(lspFileUri)
        if (!uri) return

        const items = host.lsp.getDiagnostics(uri)
        const json = JSON.stringify(items)
        if (json === this.prevJson) return
        this.prevJson = json

        view.dispatch({ effects: setDiags.of(resolve(view, items)) })
      })
    }

    destroy() {
      if (this.pending > -1) cancelAnimationFrame(this.pending)
    }
  })
}

function createDiagHover(appContext: AppContext | null) {
  return hoverTooltip((view, pos): Tooltip | null => {
    const { items } = view.state.field(diagField)
    const hits = items.filter((d) => pos >= d.from && pos <= d.to)
    if (hits.length === 0) return null

    return {
      pos,
      above: true,
      create() {
        const container = document.createElement('div')
        const renderers: VueRenderer[] = []
        for (const d of hits) {
          const renderer = new VueRenderer(DiagnosticTooltipVue, {
            message: d.message,
            severity: d.severity,
            source: d.source,
          }, { appContext })
          renderers.push(renderer)
          container.appendChild(renderer.dom)
        }
        return {
          dom: container,
          destroy() {
            for (const r of renderers) r.destroy()
          },
        }
      },
    }
  })
}

const diagTheme = EditorView.baseTheme({
  '.cm-diag-error': { textDecoration: 'wavy underline var(--color-on-danger)', textUnderlineOffset: '3px' },
  '.cm-diag-warning': { textDecoration: 'wavy underline var(--color-on-warning)', textUnderlineOffset: '3px' },
  '.cm-diag-info': { textDecoration: 'wavy underline var(--color-on-accent)', textUnderlineOffset: '3px' },
  '.cm-diag-hint': { textDecoration: 'wavy underline var(--color-fg-faint)', textUnderlineOffset: '3px' },
})

export function lspDiagnosticsExtension(host: EditorHost, appContext: AppContext | null): Extension {
  return [diagField, createSyncPlugin(host), createDiagHover(appContext), diagTheme]
}

export function getResolvedDiagnostics(view: EditorView) {
  return view.state.field(diagField, false)?.items ?? []
}
