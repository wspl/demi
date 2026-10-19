// `bun xtask` and `bun browse` build from a shell whose PATH may lack rustup's
// `~/.cargo/bin`, such as an agent's: they find cargo where rustup installs it,
// and say where they looked when it is nowhere. Costs a few milliseconds.
import { afterAll, expect, test } from 'bun:test'
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { findCargo } from '../cargo'

const root = mkdtempSync(join(tmpdir(), 'cargo-search-'))
afterAll(() => rmSync(root, { recursive: true, force: true }))

/** A directory holding an executable `cargo`. */
function withCargo(name: string): string {
  const directory = join(root, name)
  mkdirSync(directory, { recursive: true })
  writeFileSync(join(directory, 'cargo'), '#!/bin/sh\n')
  chmodSync(join(directory, 'cargo'), 0o755)
  return directory
}

const empty = join(root, 'empty')
mkdirSync(empty)

test.skipIf(process.platform === 'win32')('cargo is found on PATH, then in CARGO_HOME, then in the home directory', () => {
  const onPath = withCargo('path')
  const cargoHome = join(root, 'cargo-home')
  withCargo('cargo-home/bin')
  const home = join(root, 'home')
  withCargo('home/.cargo/bin')
  expect(findCargo({ path: onPath, cargoHome, home })).toBe(join(onPath, 'cargo'))
  expect(findCargo({ path: empty, cargoHome, home })).toBe(join(cargoHome, 'bin', 'cargo'))
  expect(findCargo({ path: empty, cargoHome: join(root, 'none'), home })).toBe(join(home, '.cargo', 'bin', 'cargo'))
})

test('no cargo anywhere names the places looked and how to install it', () => {
  const cargoHome = join(root, 'no-cargo-home')
  expect(() => findCargo({ path: empty, cargoHome, home: empty })).toThrow(
    `cargo was not found. Looked in:\n  PATH\n  ${join(cargoHome, 'bin')}\n  ${join(empty, '.cargo', 'bin')}\nInstall Rust with rustup: https://rustup.rs`,
  )
})
