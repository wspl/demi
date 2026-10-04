export interface ScrollbarAxisInput {
  viewportSize: number
  scrollSize: number
  scrollOffset: number
  trackSize: number
  minThumbSize: number
}

export interface ScrollbarAxisOutput {
  thumbSize: number
  thumbOffset: number
  scrollable: boolean
}

/**
 * A drawn scrollbar along one axis: whether there is anything to scroll, and
 * the thumb's length and its offset in the track for the scroll position.
 */
export function computeScrollbarAxis(input: ScrollbarAxisInput): ScrollbarAxisOutput {
  const { viewportSize, scrollSize, scrollOffset, trackSize, minThumbSize } = input

  if (scrollSize <= viewportSize || trackSize <= 0) {
    return {
      thumbSize: Math.max(trackSize, 0),
      thumbOffset: 0,
      scrollable: false,
    }
  }

  const visibleRatio = viewportSize / scrollSize
  const rawThumbSize = trackSize * visibleRatio
  const thumbSize = Math.min(trackSize, Math.max(minThumbSize, rawThumbSize))
  const maxScroll = scrollSize - viewportSize
  const maxThumbOffset = trackSize - thumbSize
  const thumbOffset = maxScroll <= 0 ? 0 : (scrollOffset / maxScroll) * maxThumbOffset

  return {
    thumbSize,
    thumbOffset: Math.max(0, Math.min(maxThumbOffset, thumbOffset)),
    scrollable: true,
  }
}
