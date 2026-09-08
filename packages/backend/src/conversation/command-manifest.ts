const transpiler = new Bun.Transpiler({ loader: 'ts', target: 'browser' })
const transpiled = new Map<string, string>()

/** The backend compiles each immutable command module once. */
export function transpileCommandModule(source: string): string {
  let javascript = transpiled.get(source)
  if (javascript === undefined) {
    javascript = transpiler.transformSync(source)
    transpiled.set(source, javascript)
  }
  return javascript
}
