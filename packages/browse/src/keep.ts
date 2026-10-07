// `keep` (browse.md § A call): an object whose properties a script sets for
// a later call. The server holds it between calls; when the tool's code
// changes, or a call runs into its time limit, the server that ends leaves
// it in the slot's folder for the next one, as the browser's pages stay. A
// value goes as the structured clone algorithm copies it, so a Map or a Date
// comes back as it was, and a function or a Playwright object, which only
// the process that made it can use, does not.
import { existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { deserialize, serialize } from 'node:v8'
import { z } from 'zod'

export type Keep = Record<string, unknown>

const keptSchema = z.record(z.string(), z.unknown())

/** Leaves `keep` in the file `path`; answers the names of the properties that cannot be copied, and why. */
export function saveKeep(path: string, keep: Keep): string[] {
  const copied: Keep = {}
  const lost: string[] = []
  for (const [name, value] of Object.entries(keep)) {
    try {
      serialize(value)
      copied[name] = value
    } catch (error) {
      lost.push(`keep.${name}: ${(error instanceof Error ? error.message : String(error)).replace(/\.$/, '')}`)
    }
  }
  writeFileSync(path, serialize(copied))
  return lost
}

/** What the last server left in `path`, which is used once, or an empty object. */
export function restoreKeep(path: string): Keep {
  if (!existsSync(path)) {
    return {}
  }
  const kept = keptSchema.parse(deserialize(readFileSync(path)))
  rmSync(path)
  return kept
}
