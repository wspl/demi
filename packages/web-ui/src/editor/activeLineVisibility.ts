import { StateEffect, StateField, type Extension } from '@codemirror/state'
import { EditorView } from '@codemirror/view'

const INTERACTED_CLASS = 'cm-user-interacted'
const EMPTY_ATTRS: Record<string, string> = {}

const markInteractedEffect = StateEffect.define<boolean>()

const interactedField = StateField.define<boolean>({
  create() {
    return false
  },
  update(value, tr) {
    for (const effect of tr.effects) {
      if (effect.is(markInteractedEffect)) return effect.value
    }
    return value
  },
})

function dispatchMarkInteracted(view: EditorView) {
  if (view.state.field(interactedField, false)) return
  view.dispatch({ effects: markInteractedEffect.of(true) })
}

export function activeLineVisibilityExtension(): Extension {
  return [
    interactedField,
    EditorView.editorAttributes.from(interactedField, (interacted) => (
      interacted ? { class: INTERACTED_CLASS } : EMPTY_ATTRS
    )),
    EditorView.domEventHandlers({
      mousedown(_event, view) {
        dispatchMarkInteracted(view)
        return false
      },
      touchstart(_event, view) {
        dispatchMarkInteracted(view)
        return false
      },
      keydown(_event, view) {
        dispatchMarkInteracted(view)
        return false
      },
      focus(_event, view) {
        dispatchMarkInteracted(view)
        return false
      },
    }),
  ]
}

export function markActiveLineInteracted(view: EditorView) {
  dispatchMarkInteracted(view)
}

export { INTERACTED_CLASS as activeLineInteractedClass }
