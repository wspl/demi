import { expect, test } from 'bun:test'
import * as agent from '../index'

test('agent root entry does not export node-only stdio transports', () => {
  expect('createStdioClientTransport' in agent).toBe(false)
  expect('createStdioServerTransport' in agent).toBe(false)
  expect('createWebSocketClientTransport' in agent).toBe(true)
})
