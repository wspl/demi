import { describe, expect, it } from 'bun:test'
import { retryAtFromUpstream } from '../upstream'

const received = '2026-09-18T14:00:00.000Z'

describe('retryAtFromUpstream', () => {
  it('reads an absolute reset from a stream event, wherever the vendor nests it', () => {
    const at = new Date(1790062659 * 1000).toISOString()
    expect(retryAtFromUpstream(JSON.stringify({ type: 'error', error: { resets_at: 1790062659, resets_in_seconds: 5 } }), received)).toBe(at)
    expect(retryAtFromUpstream(JSON.stringify({ status: 429, headers: {}, body: { error: { resets_at: 1790062659 } } }), received)).toBe(at)
  })

  it('counts a relative wait from when the failure was recorded', () => {
    expect(retryAtFromUpstream(JSON.stringify({ error: { resets_in_seconds: 90 } }), received)).toBe('2026-09-18T14:01:30.000Z')
    expect(retryAtFromUpstream(JSON.stringify({ status: 429, headers: { 'retry-after': '120' }, body: 'slow down' }), received)).toBe('2026-09-18T14:02:00.000Z')
  })

  it('names no time when the vendor named none, or the record is not JSON', () => {
    expect(retryAtFromUpstream(JSON.stringify({ status: 401, headers: {}, body: 'nope' }), received)).toBeNull()
    expect(retryAtFromUpstream('not json', received)).toBeNull()
    expect(retryAtFromUpstream(undefined, received)).toBeNull()
    expect(retryAtFromUpstream(JSON.stringify({ error: { resets_at: 'soon' } }), received)).toBeNull()
  })
})
