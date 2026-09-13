import type { AppContext } from 'vue'
import { StateField, StateEffect, type Extension } from '@codemirror/state'
import { showTooltip, type Tooltip, ViewPlugin, type ViewUpdate, keymap } from '@codemirror/view'
import type { EditorHost } from '../host/types'
import { lspFileUri, syncChange } from './docSync'
import { findPluginForResource, toLspPosition } from './utils'
import { VueRenderer } from '../render/vueRenderer'
import SignatureTooltipVue from '../components/editor/SignatureTooltip.vue'

interface SignatureData {
  signatures: Array<{
    label: string
    documentation?: string | { kind: string; value: string }
    parameters?: Array<{ label: string | [number, number]; documentation?: string }>
    activeParameter?: number
  }>
  activeSignature?: number
  activeParameter?: number
}

interface SigState {
  data: SignatureData
  pos: number
}

const setSig = StateEffect.define<SigState | null>()

function createSigField(appContext: AppContext | null) {
  return StateField.define<SigState | null>({
    create: () => null,
    update(state, tr) {
      for (const e of tr.effects) {
        if (e.is(setSig)) return e.value
      }
      if (!state) return null
      if (tr.docChanged) {
        return { ...state, pos: tr.changes.mapPos(state.pos) }
      }
      const head = tr.state.selection.main.head
      if (head < state.pos) return null
      return state
    },
    provide: (f) => showTooltip.from(f, (state): Tooltip | null => {
      if (!state) return null
      return {
        pos: state.pos,
        above: true,
        create() {
          if (state.data.signatures.length === 0) {
            return { dom: document.createElement('div') }
          }

          const renderer = new VueRenderer(SignatureTooltipVue, {
            signatures: state.data.signatures,
            activeSignature: state.data.activeSignature,
            activeParameter: state.data.activeParameter,
          }, { appContext })

          return {
            dom: renderer.dom,
            destroy: () => renderer.destroy(),
          }
        },
      }
    }),
  })
}

function createSigPlugin(host: EditorHost, sigField: StateField<SigState | null>) {
  return ViewPlugin.fromClass(class {
    private pending: { pos: number; drop: boolean } | null = null

    update(update: ViewUpdate) {
      const isActive = !!update.view.state.field(sigField)

      if (!update.docChanged && !update.selectionSet) return
      if (!update.docChanged && !isActive) return

      const resourceUri = update.view.state.facet(lspFileUri)
      const target = findPluginForResource(host, resourceUri)
      if (!target?.capabilities.signatureHelpProvider) return

      const triggers = target.capabilities.signatureHelpProvider.triggerCharacters ?? []
      let triggered = false
      if (update.docChanged) {
        update.changes.iterChanges((_fromA, _toA, _fromB, _toB, inserted) => {
          const text = inserted.toString()
          if (text && triggers.some((ch: string) => text.includes(ch))) triggered = true
        })
      }

      if (!triggered && !isActive) return

      const pos = update.view.state.selection.main.head
      if (this.pending) this.pending.drop = true
      const req = this.pending = { pos, drop: false }

      if (update.docChanged) {
        syncChange(host, resourceUri, update.view.state.doc.toString())
      }

      host.lsp.sendRequest(target, 'textDocument/signatureHelp', {
        textDocument: { uri: host.lsp.toDocumentUri(target, resourceUri) },
        position: toLspPosition(update.view.state.doc, pos),
        context: {
          triggerKind: triggered ? 2 : 3,
          isRetrigger: isActive,
        },
      }).then((result) => {
        const signatureResult = result as SignatureData | null
        if (req.drop) return
        if (signatureResult?.signatures.length) {
          update.view.dispatch({ effects: setSig.of({ data: signatureResult, pos: req.pos }) })
        } else {
          update.view.dispatch({ effects: setSig.of(null) })
        }
      })
    }

    destroy() {
      if (this.pending) this.pending.drop = true
    }
  })
}

const sigKeymap = (sigField: StateField<SigState | null>) => keymap.of([{
  key: 'Escape',
  run(view) {
    if (view.state.field(sigField)) {
      view.dispatch({ effects: setSig.of(null) })
      return true
    }
    return false
  },
}])

export function lspSignatureExtension(host: EditorHost, appContext: AppContext | null): Extension {
  const sigField = createSigField(appContext)
  return [sigField, createSigPlugin(host, sigField), sigKeymap(sigField)]
}
