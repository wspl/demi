package process

//revive:disable:unused-parameter // API checkpoint: stub parameter names document the boundary.

// Tail keeps the last limit bytes of a stream.
type Tail struct{}

// NewTail creates a tail with the supplied byte limit.
func NewTail(limit int) *Tail { panic("not written: r-process") }

// Push retains the newest bytes within the limit.
func (t *Tail) Push(chunk []byte) { panic("not written: r-process") }

// Bytes returns the retained bytes, transferring their ownership to the caller.
func (t *Tail) Bytes() []byte { panic("not written: r-process") }

// Text returns the bytes as text, replacing invalid or cut UTF-8 characters.
func (t *Tail) Text() string { panic("not written: r-process") }
