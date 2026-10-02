package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

// LineCounts returns lines added and removed using an exact line Myers diff.
// A nil side is absent; binary or non-UTF-8 content yields zero counts.
// Callers represent unread or oversized content according to their own limits.
func LineCounts(before, after []byte) (added, removed uint64) { panic("not written: r-process") }
