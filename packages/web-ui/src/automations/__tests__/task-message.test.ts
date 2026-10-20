import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, test } from 'bun:test'

// A task's message as the user writes it: the sheet keeps an unsaved draft
// whatever answer of a save replaces the task meanwhile, and the editor marks
// a message of only spaces as missing, as it refuses to run it. One Vue app
// on a happy-dom page; a few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { createApp, h, nextTick, ref } = await import('vue')
const { createOverlayStore } = await import('../../overlay/overlayStore')
const { default: AutomationTaskDialog } = await import('../AutomationTaskDialog.vue')
const { default: AutomationEditor } = await import('../AutomationEditor.vue')
type AutomationTask = import('../types').AutomationTask
type Automation = import('../types').Automation

let unmount: (() => void) | null = null
afterEach(() => {
  unmount?.()
  unmount = null
  document.body.innerHTML = ''
})

function mount(render: () => ReturnType<typeof h>): void {
  const root = document.createElement('div')
  document.body.append(root)
  const app = createApp({ render })
  app.mount(root)
  unmount = () => app.unmount()
}

async function settled(): Promise<void> {
  for (let tick = 0; tick < 3; tick++) {
    await Promise.resolve()
    await nextTick()
  }
}

const task: AutomationTask = {
  id: 'review',
  time: '09:00',
  days: ['mon'],
  day: { kind: 'date', date: 1 },
  month: 1,
  holidays: null,
  prompt: '',
}

test('the sheet keeps the message being written when a save replaces its task', async () => {
  const shown = ref<AutomationTask>(task)
  const overlayStore = createOverlayStore()
  mount(() => h(AutomationTaskDialog, { isOpen: true, task: shown.value, unit: 'weeks', holidayCountries: [], overlayStore }))
  await settled()
  const message = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')!
  message.value = 'Review the open invoices.'
  message.dispatchEvent(new Event('input'))
  await settled()

  // The time's save answers while the message is written: the same task
  // with its new time, then the same again.
  shown.value = { ...task, time: '10:00' }
  await settled()
  shown.value = { ...task, time: '10:00' }
  await settled()
  expect(message.value).toBe('Review the open invoices.')
})

test('a message of only spaces and line breaks shows as missing', async () => {
  const automation: Automation = {
    id: 'a1',
    name: 'Invoices',
    trigger: {
      kind: 'time',
      schedule: { kind: 'repeat', repeat: { every: 1, unit: 'weeks', start: null }, tasks: [{ ...task, prompt: '  \n ' }] },
    },
    timeZone: 'UTC',
    maxRunsPerDay: null,
    target: { kind: 'new', hostId: 'cloud', projectId: null },
    prompt: '',
    model: null,
    paused: false,
    pausedBecause: null,
    address: null,
    stripeSigning: false,
    nextAt: null,
    runs: [],
  }
  const overlayStore = createOverlayStore()
  mount(() => h(AutomationEditor, {
    automation,
    hosts: [{ id: 'cloud', name: 'Cloud', kind: 'cloud', state: 'online', mark: null, projects: [] }],
    conversations: [],
    timeZones: ['UTC'],
    holidayCountries: [],
    providers: [],
    models: {},
    overlayStore,
  }))
  await settled()
  expect(document.body.textContent).toContain('No message')
})
