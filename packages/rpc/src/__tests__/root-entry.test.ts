import { expect, test } from 'bun:test'
import * as rpc from '../index'

test('rpc root entry does not export node-only stdio transports', () => {
  expect('createStdioClientTransport' in rpc).toBe(false)
  expect('createStdioHostTransport' in rpc).toBe(false)
  expect('createWebSocketClientTransport' in rpc).toBe(true)
})
