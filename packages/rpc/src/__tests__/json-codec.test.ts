import { expect, test } from 'bun:test'
import { parseRpcJson, stringifyRpcJson } from '../index'

test('RPC JSON codec preserves BigInt metadata and Uint8Array values', () => {
  const encoded = stringifyRpcJson({
    metadata: { count: 42n },
    bytes: new Uint8Array([1, 2, 3]),
    empty: new Uint8Array(),
    one: new Uint8Array([255]),
    two: new Uint8Array([254, 253]),
  })

  const decoded = parseRpcJson<{
    metadata: { count: bigint }
    bytes: Uint8Array
    empty: Uint8Array
    one: Uint8Array
    two: Uint8Array
  }>(encoded)

  expect(decoded.metadata.count).toBe(42n)
  expect(decoded.bytes).toBeInstanceOf(Uint8Array)
  expect([...decoded.bytes]).toEqual([1, 2, 3])
  expect([...decoded.empty]).toEqual([])
  expect([...decoded.one]).toEqual([255])
  expect([...decoded.two]).toEqual([254, 253])
})
