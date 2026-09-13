import { afterEach, expect, test } from 'bun:test'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import process from 'node:process'
import { resolveWireLogDir } from '../wire-log'

const DEFAULT_DIR = join(tmpdir(), 'demi-claude-wire')

afterEach(() => {
  delete process.env.DEMI_CLAUDE_WIRE_LOG
  delete process.env.DEMI_CLAUDE_WIRE_LOG_DIR
})

test('the wire log is on by default and relocatable', () => {
  expect(resolveWireLogDir()).toBe(DEFAULT_DIR)
  process.env.DEMI_CLAUDE_WIRE_LOG = 'true'
  expect(resolveWireLogDir()).toBe(DEFAULT_DIR)
  process.env.DEMI_CLAUDE_WIRE_LOG_DIR = '/tmp/demi-wire-elsewhere'
  expect(resolveWireLogDir()).toBe('/tmp/demi-wire-elsewhere')
})

test('both spellings of off turn the wire log off', () => {
  process.env.DEMI_CLAUDE_WIRE_LOG = '0'
  expect(resolveWireLogDir()).toBeNull()
  process.env.DEMI_CLAUDE_WIRE_LOG = 'false'
  expect(resolveWireLogDir()).toBeNull()
})

test('a value that is neither on nor off is a configuration error', () => {
  process.env.DEMI_CLAUDE_WIRE_LOG = 'False'
  expect(() => resolveWireLogDir()).toThrow(/DEMI_CLAUDE_WIRE_LOG/)
  process.env.DEMI_CLAUDE_WIRE_LOG = 'yes'
  expect(() => resolveWireLogDir()).toThrow(/DEMI_CLAUDE_WIRE_LOG/)
})
