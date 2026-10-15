import { expect, test } from 'bun:test'
import { effectScope, nextTick, ref } from 'vue'
import { useDevicePairing } from '../../devices/pairing'
import { HostFiles, type Answered } from '../../files/file-cache'
import { SimulatedWatch } from '../../files/memory-source'
import { WatchChannel } from '../../files/watch-channel'
import { keptSource } from '../../files/kept-source'
import { usePathCompletion } from '../../files/path-completion'
import { FileBrowserError, type FileBrowserEntry } from '../../files/types'
import {
  openingChoice,
  useDeviceProjectAction,
  usePairedSelection,
  type WorkspaceDevice,
} from '../workspace'

// Pure functions over small arrays, one stubbed claim and an in-memory
// listing the test answers: a few milliseconds.

const devices: WorkspaceDevice[] = [
  { id: 'laptop', name: 'laptop', state: 'online', mark: null },
  { id: 'build-01', name: 'build-01', state: 'offline', mark: null },
]

test('New project opens on the Cloud the first time, with no device chosen', () => {
  expect(openingChoice(undefined, devices)).toEqual({ kind: 'cloud' })
})

test('New project opens on the last choice of kind and device, offline or not', () => {
  expect(openingChoice({ kind: 'device', deviceId: 'build-01' }, devices)).toEqual({
    kind: 'device',
    deviceId: 'build-01',
  })
  // The device last chosen stays while the Cloud is chosen, for a switch to Device.
  expect(openingChoice({ kind: 'cloud', deviceId: 'laptop' }, devices)).toEqual({
    kind: 'cloud',
    deviceId: 'laptop',
  })
})

test('a device the user no longer has leaves Device with no device chosen', () => {
  expect(openingChoice({ kind: 'device', deviceId: 'removed' }, devices)).toEqual({
    kind: 'device',
  })
})

test('a device paired from beside the device menu is selected there once the menu lists it', async () => {
  const listed = ref<WorkspaceDevice[]>([...devices])
  const selected = ref('laptop')
  const scope = effectScope()
  const pairing = scope.run(() => {
    const paired = usePairedSelection(
      () => listed.value,
      (id) => {
        selected.value = id
      },
    )
    const pairing = useDevicePairing(async () => ({
      ok: true,
      device: { id: 'studio', name: 'studio' },
    }))
    pairing.open(paired)
    return pairing
  })!
  pairing.phase.value = { kind: 'code' }
  await pairing.submit('ABCD-1234')
  await nextTick()
  // The claim has answered, but the page has not brought the device yet.
  expect(selected.value).toBe('laptop')

  listed.value = [...listed.value, { id: 'studio', name: 'studio', state: 'online', mark: null }]
  await nextTick()
  expect(selected.value).toBe('studio')
  scope.stop()
})

test('a device project\'s button says Add Project for a directory that exists and Create Project for one Demi makes, from the completion\'s own listings', async () => {
  const folder = (name: string): FileBrowserEntry => ({ name, isDirectory: true })
  const tree: Record<string, FileBrowserEntry[]> = {
    '/Users/alex': [folder('Projects')],
    '/Users/alex/Projects': [folder('demi'), { name: 'notes.md', isDirectory: false }],
    '/Users/alex/Projects/demi': [folder('src')],
  }
  const listed: string[] = []
  const pending: (() => void)[] = []
  const files = new HostFiles()
  const watched = new SimulatedWatch()
  const source = keptSource({
    platform: 'macos',
    home: '/Users/alex',
    list(path: string): Promise<Answered<FileBrowserEntry[]>> {
      const readAt = watched.readAt()
      listed.push(path)
      return new Promise((resolve, reject) => {
        pending.push(() => {
          const entries = tree[path]
          if (entries) {
            resolve({ value: entries, readAt })
          } else {
            reject(new FileBrowserError('not-found'))
          }
        })
      })
    },
  }, { files, watch: new WatchChannel(files, watched.open) })
  const path = ref('')
  const scope = effectScope()
  const { completion, action } = scope.run(() => ({
    completion: usePathCompletion({ source: () => source, base: () => undefined, kind: () => 'directory' }),
    action: useDeviceProjectAction({ source: () => source, path: () => path.value }),
  }))!
  /** Types `text` with the caret at its end, as the field's completion follows it. */
  function type(text: string): void {
    path.value = text
    completion.follow(text, text.length)
  }
  /** Answers every listing asked for so far, and lets their results land. */
  async function answer(): Promise<void> {
    await nextTick()
    for (const settle of pending.splice(0)) {
      settle()
    }
    await new Promise((resolve) => setTimeout(resolve, 0))
  }

  type('/Users/alex/Projects/demi/')
  await answer()
  expect(action.value).toBe('add')
  type('/Users/alex/Projects/new')
  await answer()
  expect(action.value).toBe('create')
  // A file of that name is not a directory to add.
  type('/Users/alex/Projects/notes.md')
  await answer()
  expect(action.value).toBe('create')
  type('/Users/alex/Projects/demi')
  await answer()
  expect(action.value).toBe('add')
  // A directory whose listing finds it gone is made.
  type('/Users/alex/Projects/new/')
  await answer()
  expect(action.value).toBe('create')
  // While a listing is on its way the button keeps what it said, then follows it.
  type('/Users/alex/')
  await nextTick()
  expect(action.value).toBe('create')
  await answer()
  expect(action.value).toBe('add')
  type('/Users/alex/Documents/')
  await nextTick()
  expect(action.value).toBe('add')
  await answer()
  expect(action.value).toBe('create')
  // The button reads the listings the completion asked for, and asks for none of its own.
  expect(listed).toEqual(['/Users/alex/Projects/demi', '/Users/alex/Projects', '/Users/alex/Projects/new', '/Users/alex', '/Users/alex/Documents'])
  scope.stop()
})
