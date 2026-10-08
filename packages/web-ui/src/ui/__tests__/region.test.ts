import { expect, test } from 'bun:test'
import { inwardAlignment } from '../region'

// A window 1400 px wide: the sidebar 0–260, the conversation 260–1000, the work panel 1000–1400.
const SIDEBAR = { left: 0, width: 260 }
const CONVERSATION = { left: 260, width: 740 }
const WORK_PANEL = { left: 1000, width: 400 }

test('a menu grows toward the inside of the region its trigger sits in', () => {
  const cases = [
    // The conversation header's directory button, at the pane's end: the menu ends where it ends.
    { trigger: { left: 880, width: 100 }, region: CONVERSATION, alignment: 'end' },
    // The rename button after the title, at the pane's start.
    { trigger: { left: 300, width: 28 }, region: CONVERSATION, alignment: 'start' },
    // A trigger at the work panel's start lies in the window's end half but its panel's start half.
    { trigger: { left: 1010, width: 28 }, region: WORK_PANEL, alignment: 'start' },
    { trigger: { left: 1360, width: 28 }, region: WORK_PANEL, alignment: 'end' },
    // A trigger that fills its region, as the sidebar's account row does, is centred: it keeps the start.
    { trigger: { left: 0, width: 260 }, region: SIDEBAR, alignment: 'start' },
  ] as const
  for (const { trigger, region, alignment } of cases)
    expect({ trigger, alignment: inwardAlignment(trigger, region) }).toEqual({ trigger, alignment })
})
