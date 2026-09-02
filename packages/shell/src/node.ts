// The Host contract over Node, shipped as `@demicodes/shell/node` so the
// root entry stays free of Node: `nodeFileSystem` — the backend machine's
// own filesystem, the backing of the store-backed Host — and `LocalHost`,
// the whole contract over this Node process's machine (child processes, a
// directory-fd cwd, the real identity), which tests run against a real
// directory. The product's machines are reached through the runner.
export { nodeFileSystem } from './node/file-system'
export { LocalHost, type LocalHostOptions } from './node/local-host'
