import { afterEach, expect, test } from 'bun:test'
import { createRenderer, defineComponent, h, nextTick, ref, type App, type PropType } from 'vue'
import { answerInDialog, type WorkDialog } from '../../settings/dialog-work'
import { dismissToast, toasts } from '../../infra/toast'
import { operations, provideRowTarget, useOpening, useOperationControl, type Operation } from '../operations'

// What the page does with a control's operation: the page keeps it, not the
// control, so a control drawn again shows it and a press meanwhile starts
// nothing; a row shows its target's work; one group's operations exclude each
// other; and a finished operation answers only where it was started. Controls
// and rows press and show through the same functions Button, IconButton,
// MenuItem and SettingsRow use, rendered by Vue over plain objects; a few
// milliseconds.

interface Node {
  parent: Node | null
  children: Node[]
}
const node = (): Node => ({ parent: null, children: [] })
const { createApp } = createRenderer<Node, Node>({
  patchProp: () => {},
  insert(child, parent, anchor) {
    child.parent = parent
    parent.children.splice(anchor ? parent.children.indexOf(anchor) : parent.children.length, 0, child)
  },
  remove(child) {
    child.parent?.children.splice(child.parent.children.indexOf(child), 1)
  },
  createElement: node,
  createText: node,
  createComment: node,
  setText: () => {},
  setElementText: () => {},
  parentNode: (child) => child.parent,
  nextSibling: (child) => child.parent?.children[child.parent.children.indexOf(child) + 1] ?? null,
})

/** A promise the test settles when it chooses. */
function later() {
  const { promise, resolve, reject } = Promise.withResolvers<void>()
  return { promise, resolve, reject }
}

/** Lets settled work end its operation. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 5; turn++) {
    await Promise.resolve()
  }
  await nextTick()
}

let apps: App[] = []
let errors: unknown[] = []
afterEach(() => {
  for (const app of apps) {
    app.unmount()
  }
  apps = []
  errors = []
  for (const toast of [...toasts]) {
    dismissToast(toast.id)
  }
})

/** A drawn control: what it shows, and its press, as a button's click. */
interface Drawn {
  running: () => boolean
  blocked: () => boolean
  showsSpinner: () => boolean
  press: (listener: unknown) => void
}

/** Renders `root`; each control it draws reports itself in `drawn`, the latest last. */
function mount(root: () => ReturnType<typeof h>): void {
  const app = createApp({ render: root })
  app.config.errorHandler = (error) => errors.push(error)
  app.mount(node())
  apps.push(app)
}

function control(drawn: Drawn[], rowAction = false) {
  return defineComponent({
    props: { operation: Object as PropType<Operation> },
    setup(props) {
      const { running, blocked, showsSpinner, press } = useOperationControl(() => props.operation, rowAction)
      drawn.push({ running: () => running.value, blocked: () => blocked.value, showsSpinner: () => showsSpinner.value, press })
      return () => h('span')
    },
  })
}

/** A row of `target`, as SettingsRow is one, reporting whether it shows the spinner. */
function row(target: string, working: Record<string, () => boolean>) {
  return defineComponent({
    setup(_, { slots }) {
      const shown = provideRowTarget(() => target)
      working[target] = () => shown.value
      return () => h('div', slots.default?.())
    },
  })
}

test('a control drawn again while its operation runs shows it working, and a press then starts nothing', async () => {
  const drawn: Drawn[] = []
  const Control = control(drawn)
  const removing = { action: 'remove', target: 'project:app' }
  const shown = ref(true)
  mount(() => (shown.value ? h(Control, { operation: removing }) : h('span')))
  const wait = later()
  let presses = 0
  const remove = () => {
    presses++
    return wait.promise
  }
  drawn.at(-1)!.press(remove)
  expect(drawn.at(-1)!.running()).toBe(true)

  // The menu closes and opens again: a new control, the same operation.
  shown.value = false
  await nextTick()
  shown.value = true
  await nextTick()
  expect(drawn).toHaveLength(2)
  expect(drawn.at(-1)!.running()).toBe(true)
  drawn.at(-1)!.press(remove)
  expect(presses).toBe(1)

  wait.resolve()
  await settle()
  expect(drawn.at(-1)!.running()).toBe(false)
  drawn.at(-1)!.press(remove)
  expect(presses).toBe(2)
})

test('a dialog closed while its Save works leaves the work on its row, and opened again shows Save working', async () => {
  const drawn: Drawn[] = []
  const working: Record<string, () => boolean> = {}
  const Save = control(drawn)
  const Row = row('credential:github', working)
  const open = ref(true)
  const saving = { action: 'save', target: 'credential:github' }
  mount(() => h(Row, null, () => (open.value ? [h(Save, { operation: saving })] : [])))
  const wait = later()
  drawn.at(-1)!.press(() => wait.promise)
  expect(working['credential:github']!()).toBe(true)

  // Cancel closes the dialog; the request goes on, and its row shows it.
  open.value = false
  await nextTick()
  expect(working['credential:github']!()).toBe(true)
  open.value = true
  await nextTick()
  expect(drawn.at(-1)!.running()).toBe(true)

  wait.resolve()
  await settle()
  expect(working['credential:github']!()).toBe(false)
  expect(drawn.at(-1)!.running()).toBe(false)
})

test('each row shows the work on its own target only, and leaves its row actions without a spinner of their own', async () => {
  const drawn: Drawn[] = []
  const working: Record<string, () => boolean> = {}
  const Action = control(drawn, true)
  const Alice = row('user:alice', working)
  const Bob = row('user:bob', working)
  mount(() =>
    h('div', [
      h(Alice, null, () => [h(Action, { operation: { action: 'disable', target: 'user:alice' } })]),
      h(Bob, null, () => [h(Action, { operation: { action: 'disable', target: 'user:bob' } })]),
    ]),
  )
  const [alice, bob] = drawn
  const wait = later()
  alice!.press(() => wait.promise)
  expect(working['user:alice']!()).toBe(true)
  expect(working['user:bob']!()).toBe(false)
  // The row shows it, not the row action itself.
  expect(alice!.running()).toBe(true)
  expect(alice!.showsSpinner()).toBe(false)
  expect(bob!.blocked()).toBe(false)
  wait.resolve()
  await settle()
  expect(working['user:alice']!()).toBe(false)
})

test('the operations of one group exclude each other on their target, and Cancel stays free', async () => {
  const drawn: Drawn[] = []
  const Control = control(drawn)
  const save = { action: 'save', target: 'profile:p1', group: 'edit' }
  const remove = { action: 'delete', target: 'profile:p1', group: 'edit' }
  const otherSave = { action: 'save', target: 'profile:p2', group: 'edit' }
  mount(() =>
    h('div', [
      h(Control, { operation: save }),
      h(Control, { operation: remove }),
      h(Control),
      h(Control, { operation: otherSave }),
    ]),
  )
  const [saveButton, deleteButton, cancelButton, otherButton] = drawn
  const wait = later()
  saveButton!.press(() => wait.promise)
  expect(saveButton!.running()).toBe(true)
  expect(deleteButton!.running()).toBe(false)
  expect(deleteButton!.blocked()).toBe(true)
  let deleted = false
  deleteButton!.press(() => {
    deleted = true
  })
  expect(deleted).toBe(false)
  expect(cancelButton!.blocked()).toBe(false)
  expect(otherButton!.blocked()).toBe(false)
  wait.resolve()
  await settle()
  expect(deleteButton!.blocked()).toBe(false)
})

test('a rejected operation is reported as an event handler’s failure is, and ends', async () => {
  const drawn: Drawn[] = []
  const Control = control(drawn)
  mount(() => h(Control, { operation: { action: 'remove', target: 'passkey:k1' } }))
  const wait = later()
  drawn[0]!.press(() => wait.promise)
  wait.reject(new Error('refused'))
  await settle()
  expect(drawn[0]!.running()).toBe(false)
  expect(operations.onTarget('passkey:k1')).toBe(false)
  expect(errors.map((error) => (error as Error).message)).toEqual(['refused'])
})

test('a store that starts the operation its button was pressed for joins it, and work that waits for it starts after', async () => {
  const drawn: Drawn[] = []
  const Control = control(drawn)
  const testing = { action: 'testing', target: 'provider:p', group: 'provider' }
  mount(() => h(Control, { operation: testing }))
  const wait = later()
  let started = 0
  // As the providers store does: the button's handler only asks, the store starts the operation.
  const storeTest = () =>
    operations.run(testing, () => {
      started++
      return wait.promise
    })
  drawn[0]!.press(() => {
    void storeTest()
  })
  expect(started).toBe(1)
  expect(drawn[0]!.running()).toBe(true)
  const order: string[] = []
  const after = operations.whenFree(testing).then(() => order.push('free'))
  wait.resolve()
  await after
  expect(order).toEqual(['free'])
  expect(operations.blocked(testing)).toBe(false)
})

test('an answer goes only to the dialog it was started from, and a refusal once that is gone is a toast', async () => {
  const isOpen = ref(true)
  let opening!: ReturnType<typeof useOpening>
  mount(() =>
    h(
      defineComponent({
        setup() {
          opening = useOpening(() => isOpen.value)
          return () => h('span')
        },
      }),
    ),
  )
  const stillShown = opening.shownNow()
  // Closed and opened again, as for another account: the first answer is not this opening's.
  isOpen.value = false
  await nextTick()
  isOpen.value = true
  await nextTick()
  expect(stillShown()).toBe(false)
  expect(opening.shownNow()()).toBe(true)

  // The dialog the work came from is gone: the refusal is said in a toast.
  const refusal = later()
  const gone = answerInDialog(() => null, 'Could Not Save the Credential', () => ({ message: 'in the dialog', requestId: null }), () => refusal.promise)
  refusal.reject(new Error('That name is taken.'))
  await gone
  expect(toasts.map((toast) => [toast.title, toast.message])).toEqual([['Could Not Save the Credential', 'That name is taken.']])

  // Still open: done, it closes.
  const dialog: WorkDialog = { open: true, error: null }
  await answerInDialog(() => (dialog.open ? dialog : null), 'Could Not Save the Credential', () => ({ message: '', requestId: null }), async () => {})
  expect(dialog.open).toBe(false)
})
