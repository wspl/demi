import { StateEffect } from '@codemirror/state'
import type { EditorView, ViewUpdate } from '@codemirror/view'

export const lspRefreshEffect = StateEffect.define<void>()

export function requestLspRefresh(view: EditorView) {
  view.dispatch({ effects: lspRefreshEffect.of(undefined) })
}

export function didRequestLspRefresh(update: ViewUpdate) {
  return update.transactions.some((transaction) =>
    transaction.effects.some((effect) => effect.is(lspRefreshEffect)))
}
