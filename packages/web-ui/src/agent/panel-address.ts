import { dataChanges } from './panel-changes'
import { pinnedData, shownSelection, type ShownPanel } from './panel-tabs'
import type { PanelKind } from '../plugins/page'
import type { PlaceFields } from '../plugins/place'

/**
 * The work panel's part of the page's address
 * (`docs/decisions/0014-the-work-panel-has-addresses.md`):
 *
 * ```text
 * /chat/c1                                  the panel closed
 * /chat/c1?panel=file&path=/work/main.ts    the File view at a file
 * /chat/c1?panel=t7                         the tab t7
 * ```
 *
 * `panel` names what the panel selects: a pinned tab by its kind, any other
 * tab by its id. The rest of the query is the pinned tab's place, which only
 * its kind writes and reads (`../plugins/place.ts`).
 *
 * The address and the panel's state are one thing: while a conversation is
 * shown its address says what its panel shows, and a move of the panel is a
 * move of the address, a step of the web browser's history unless it only
 * changes how the same place is seen. This module says what an address holds;
 * the page's store moves it.
 */

/** The query field that names what the panel selects. */
export const PANEL_FIELD = 'panel'

/** The panel's query: `panel` and the selected pinned tab's place, empty while the panel is closed. */
export type PanelAddress = PlaceFields

/**
 * The address of a panel `open` or closed that shows `shown`, through the
 * pages' `kinds`. A panel open on nothing names nothing, so it reads back
 * open.
 */
export function panelAddress(open: boolean, shown: ShownPanel, kinds: readonly PanelKind<unknown>[]): PanelAddress {
  if (!open) {
    return {}
  }
  const selection = shownSelection(shown.panel, kinds) ?? ''
  const kind = kinds.find((candidate) => candidate.kind === selection && candidate.pinned)
  const data = kind?.place ? kind.schema.safeParse(pinnedData(shown.pinned, kind)) : null
  if (!kind?.place || !data?.success) {
    return { [PANEL_FIELD]: selection }
  }
  const { [PANEL_FIELD]: _named, ...place } = kind.place.write(data.data)
  return { [PANEL_FIELD]: selection, ...place }
}

/** What the address names the panel selects; null while it says the panel is closed. */
export function addressSelection(address: PanelAddress): string | null {
  return address[PANEL_FIELD] ?? null
}

/**
 * The data a pinned `kind` shows at the place `address` names, from what it
 * shows now: `current` itself when the place is where it is, so its view
 * does not draw again; the kind's start when the address does not read.
 * Null when the kind has no place.
 */
export function placeData(kind: PanelKind<unknown>, address: PanelAddress, current: unknown): unknown {
  if (!kind.place || !kind.pinned) {
    return null
  }
  const parsed = kind.schema.safeParse(current)
  const shown = parsed.success ? parsed.data : kind.pinned.data()
  const { [PANEL_FIELD]: _named, ...fields } = address
  const next = kind.place.read(fields, shown) ?? kind.pinned.data()
  const changed = next !== null && typeof next === 'object' && Object.keys(dataChanges(current, { ...next })).length > 0
  return changed ? next : current
}

/** Whether two addresses say the same. */
export function sameAddress(a: PanelAddress, b: PanelAddress): boolean {
  const fields = Object.keys(a)
  return fields.length === Object.keys(b).length && fields.every((field) => a[field] === b[field])
}
