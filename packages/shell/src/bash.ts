// The just-bash engine behind the `shell_*` tools on today's runner, and the
// portable command set it carries. Everything here leaves with just-bash in
// M9 (`docs/demi-next/roadmap.md`); the package root is the command system
// and the Host contract, which run on every runtime.
export * from './environment'
export * from './portable-commands'
