import { expect, test } from 'bun:test'
import { createWindowStack, KEEPS_WINDOWS, type StackedWindow } from '../window-stack'

// The session's windows close on a press outside them, the newest first
// (`product.md`, the dock's chips under § Recovering an unfinished turn), over
// a page of plain objects instead of the DOM: a few milliseconds.

/** A place on the page, as an element is: its parent, and whether it keeps the windows open. */
class Place {
  constructor(readonly parent: Place | null = null, readonly keeps = false) {}

  closest(selectors: string): Place | null {
    expect(selectors).toBe(`[${KEEPS_WINDOWS}]`)
    for (let place: Place | null = this; place; place = place.parent) {
      if (place.keeps)
        return place
    }
    return null
  }

  within(other: Place): boolean {
    for (let place: Place | null = this; place; place = place.parent) {
      if (place === other)
        return true
    }
    return false
  }
}

const page = new Place()
const transcript = new Place(page)
const sidebar = new Place(page)
// The controls in the transcript that open or switch a window: a receipt's
// sender, a command notice's title, a thumbnail, an agent link.
const receiptSender = new Place(transcript, true)
const thumbnail = new Place(transcript, true)
const dock = new Place(page, true)
const composer = new Place(dock)
const agentsWindow = new Place(page)
const childTranscript = new Place(agentsWindow)
const viewerWindow = new Place(page)
const picture = new Place(viewerWindow)

/** The page's stack with the Agents window and the viewer, which leave it when they close, as the primitives do. */
function session() {
  const stack = createWindowStack<Place>()
  const closed: string[] = []
  let menuOpen = false
  function window(name: string, element: Place): StackedWindow<Place> {
    const entry: StackedWindow<Place> = {
      contains: (place) => place.within(element),
      otherLayerOpen: () => menuOpen,
      close: () => {
        closed.push(name)
        stack.remove(entry)
      },
    }
    return entry
  }
  return {
    stack,
    closed,
    agents: window('Agents', agentsWindow),
    viewer: window('viewer', viewerWindow),
    openMenu: () => {
      menuOpen = true
    },
  }
}

test('a press outside every window closes the newest alone, and the next press the next', () => {
  const { stack, closed, agents, viewer } = session()
  // The user opens Agents, then a screenshot in the child's transcript.
  stack.open(agents)
  stack.open(viewer)
  stack.press(transcript)
  expect(closed).toEqual(['viewer'])
  stack.press(sidebar)
  expect(closed).toEqual(['viewer', 'Agents'])
  stack.press(page)
  expect(closed).toEqual(['viewer', 'Agents'])
})

test('a press in a window, in the composer or on its chips closes nothing', () => {
  const { stack, closed, agents, viewer } = session()
  stack.open(agents)
  stack.open(viewer)
  for (const place of [composer, dock, childTranscript, picture])
    stack.press(place)
  expect(closed).toEqual([])
  expect(stack.windows.value).toEqual([agents, viewer])
})

test('a press on a control that opens or switches a window is that window’s, and closes none', () => {
  const { stack, closed, agents, viewer } = session()
  stack.open(agents)
  stack.open(viewer)
  // A receipt's sender switches the Agents window to its child; a second thumbnail shows in the viewer.
  stack.press(receiptSender)
  stack.press(thumbnail)
  expect(closed).toEqual([])
  expect(stack.windows.value).toEqual([agents, viewer])
})

test('a press while a menu is open is the menu’s, and closes no window', () => {
  const { stack, closed, agents, openMenu } = session()
  stack.open(agents)
  openMenu()
  stack.press(transcript)
  expect(closed).toEqual([])
})

test('a window that shows something new comes to the front', () => {
  const { stack, closed, agents, viewer } = session()
  stack.open(viewer)
  stack.open(agents)
  // A screenshot in the child's transcript shows in the open viewer.
  stack.open(viewer)
  stack.press(transcript)
  expect(closed).toEqual(['viewer'])
})
