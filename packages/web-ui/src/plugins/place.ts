import type { z } from 'zod'

/**
 * Where a pinned tab is, as the page's address writes it beside the panel's
 * `panel` field (`docs/decisions/0014-the-work-panel-has-addresses.md`):
 * `/chat/c1?panel=file&path=/work/src/main.ts` is the File view at that file.
 *
 * A kind declares its place with `definePlace`: the address's fields it
 * writes for the data its tab shows, the schema those fields are read back
 * with, and the data a place read back shows. The shell never reads a kind's
 * fields itself, and a query the schema does not read shows the kind's start.
 */

/** The address's query fields, without `panel`, as the page's address has them. */
export type PlaceFields = Readonly<Record<string, string>>

/** The fields a place's schema reads: each a string of the address, or absent. */
type PlaceInput = Readonly<Record<string, string | undefined>>

/** A kind's place, as the shell uses it. */
export interface PanelPlace<Data> {
  /** The address's fields of where `data` is. */
  write(data: Data): PlaceFields
  /** The data the tab shows at the place `fields` name, from what it shows now; null when they do not read. */
  read(fields: PlaceFields, current: Data): Data | null
}

/**
 * A kind's place, from the schema its fields read with. `write` answers the
 * schema's input, so what it writes reads back; `read` answers the data the
 * tab shows at the place, keeping of `current` what the address does not
 * hold.
 */
export function definePlace<Data, Place, Fields extends PlaceInput>(place: {
  schema: z.ZodType<Place, Fields>
  write(data: Data): Fields
  read(place: Place, current: Data): Data
}): PanelPlace<Data> {
  return {
    write(data) {
      const written: Record<string, string> = {}
      for (const [field, value] of Object.entries(place.write(data))) {
        if (value !== undefined) {
          written[field] = value
        }
      }
      return written
    },
    read(fields, current) {
      const parsed = place.schema.safeParse(fields)
      return parsed.success ? place.read(parsed.data, current) : null
    },
  }
}
