import { describe, expect, it } from 'bun:test'
import { computeScrollbarAxis } from '../scrollbars/metrics'

describe('computeScrollbarAxis', () => {
  it('computes thumb size and offset for a partially visible document', () => {
    expect(computeScrollbarAxis({
      viewportSize: 200,
      scrollSize: 1000,
      scrollOffset: 300,
      trackSize: 100,
      minThumbSize: 18,
    })).toEqual({
      thumbSize: 20,
      thumbOffset: 30,
      scrollable: true,
    })
  })

  it('clamps tiny documents to a disabled scrollbar', () => {
    expect(computeScrollbarAxis({
      viewportSize: 400,
      scrollSize: 400,
      scrollOffset: 0,
      trackSize: 120,
      minThumbSize: 18,
    })).toEqual({
      thumbSize: 120,
      thumbOffset: 0,
      scrollable: false,
    })
  })
})
