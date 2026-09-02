import { readFileSync } from 'node:fs'

/**
 * Build-time counterpart of importing a `*.command.ts` file with the `text`
 * attribute (`docs/demi-next/commands.md` § How a tree carries a module).
 * Bun honors the attribute; rolldown resolves the file as a module, so a
 * tsdown build of any package that declares `runtime` leaves serves
 * `*.command.ts` files as their text through this plugin.
 */
export function commandModulesAsText(): { name: string; load(id: string): string | null } {
  return {
    name: 'command-modules-as-text',
    load(id) {
      if (!id.endsWith('.command.ts')) return null
      return `export default ${JSON.stringify(readFileSync(id, 'utf8'))}`
    },
  }
}
