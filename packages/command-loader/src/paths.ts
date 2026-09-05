import { isCommandGroup, isPathArg, parseCommandInput, resolveCommand, type Command, type RootPaths } from '@demicodes/shell'

/**
 * The `RootPaths` functions for a set of trees, from the path marks on the
 * leaves' input schemas. An argv the tree cannot parse names no paths: the
 * command itself reports the error when it runs.
 */
export function rootPaths(roots: readonly Command[]): ReadonlyMap<string, RootPaths> {
  return new Map(roots.map((root) => [root.name, pathsOf(root)]))
}

function pathsOf(root: Command): RootPaths {
  return (argv) => {
    let parsed
    try {
      parsed = parseCommandInput(root, [root.name, ...argv])
    } catch {
      return []
    }
    if (parsed.help) return []
    const leaf = resolveCommand(root, parsed.path)
    if (isCommandGroup(leaf)) return []
    const paths: string[] = []
    for (const [field, schema] of Object.entries(leaf.input ?? {})) {
      if (!isPathArg(schema)) continue
      const value = parsed.values[field]
      if (typeof value === 'string') paths.push(value)
      else if (Array.isArray(value)) for (const item of value) if (typeof item === 'string') paths.push(item)
    }
    return paths
  }
}
