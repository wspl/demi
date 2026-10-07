// The footer a call ends with, composed from what the call saw; no browser.
import { expect, test } from 'bun:test'
import { composeFooter } from './footer'

test('a call ends with its screenshots, then where the page stands and what went wrong since it began', () => {
  const errors = Array.from({ length: 12 }, (_, index) => `error: failure ${index + 1}`)
  expect(composeFooter({
    shots: [
      { kind: 'timeline', path: '/s/timeline-0ms.png', detail: '1440×900 px, painted at -30 ms' },
      { kind: 'timeline', path: '/s/timeline-1500ms.png', detail: '1440×900 px, painted at 1484 ms' },
      { kind: 'shot', path: '/s/devices.png', detail: '2880×1800 px' },
    ],
    page: { url: 'http://127.0.0.1:3323/settings/devices', title: 'Settings — Demi', layer: 'dialog "Add Device"', focus: 'button "Continue"' },
    problems: { console: errors, network: ['GET 502 http://127.0.0.1:3323/api/sync 4 ms'] },
  })).toEqual([
    'timeline  /s/timeline-0ms.png     1440×900 px, painted at -30 ms',
    'timeline  /s/timeline-1500ms.png  1440×900 px, painted at 1484 ms',
    'shot      /s/devices.png          2880×1800 px',
    'Page      http://127.0.0.1:3323/settings/devices · "Settings — Demi"',
    'Layer     dialog "Add Device"',
    'Focus     button "Continue"',
    'Console   error: failure 1',
    ...errors.slice(1, 10).map((error) => `          ${error}`),
    '          … and 2 more',
    'Requests  GET 502 http://127.0.0.1:3323/api/sync 4 ms',
  ])
})

test('a call that left no browser, as down does, reports no page', () => {
  expect(composeFooter({ shots: [], page: null, problems: { console: [], network: [] } })).toEqual([])
  expect(composeFooter({
    shots: [],
    page: { url: 'about:blank', title: '', layer: null, focus: null },
    problems: { console: [], network: [] },
  })).toEqual(['Page      about:blank', 'Focus     nothing (the page)'])
})
