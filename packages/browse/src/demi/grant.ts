// `demi.grant(permission, { origin })`: grants the browser permissions for
// every origin or the one named, as Playwright's `grantPermissions` does,
// with `clipboard` standing for reading and writing it. The browser keeps
// them until it stops; `context.clearPermissions()` takes them back.
import type { Tool } from '../tool'

/** Names that stand for several of Playwright's permissions. */
const GROUPS: Record<string, string[]> = { clipboard: ['clipboard-read', 'clipboard-write'] }

export async function grant(tool: Tool, names: string | string[], options: { origin?: string } = {}): Promise<string[]> {
  const permissions = [names].flat().flatMap((name) => GROUPS[name] ?? [name])
  const context = (await tool.browser.page()).context()
  await context.grantPermissions(permissions, options)
  return permissions
}
