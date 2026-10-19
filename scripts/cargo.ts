// Finds the `cargo` the scripts build with the way rustup installs it, for a
// shell whose PATH lacks `~/.cargo/bin`, such as an agent's: rustup puts its
// proxies in `$CARGO_HOME/bin`, by default `~/.cargo/bin`, and only a login
// shell's profile adds that to PATH.
import { existsSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'

/** Where to look, with the environment and home directory of this process by default. */
export interface CargoSearch {
  path?: string
  cargoHome?: string
  home?: string
  windows?: boolean
}

/**
 * The path of `cargo`: the one on PATH, else `$CARGO_HOME/bin`'s, else
 * `~/.cargo/bin`'s. Throws an error that names where it looked and how to
 * install it when none has it.
 */
export function findCargo(search: CargoSearch = {}): string {
  const path = search.path ?? process.env.PATH ?? ''
  const cargoHome = search.cargoHome ?? process.env.CARGO_HOME
  const home = search.home ?? homedir()
  const windows = search.windows ?? process.platform === 'win32'
  const onPath = Bun.which('cargo', { PATH: path })
  if (onPath) {
    return onPath
  }
  const program = windows ? 'cargo.exe' : 'cargo'
  const directories = [cargoHome ? join(cargoHome, 'bin') : null, join(home, '.cargo', 'bin')]
    .filter((directory): directory is string => directory !== null)
  for (const directory of directories) {
    const candidate = join(directory, program)
    if (existsSync(candidate)) {
      return candidate
    }
  }
  const looked = ['PATH', ...directories].map((place) => `  ${place}`).join('\n')
  throw new Error(`cargo was not found. Looked in:\n${looked}\nInstall Rust with rustup: https://rustup.rs`)
}
