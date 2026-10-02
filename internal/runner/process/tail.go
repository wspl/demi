package process

// Tail keeps the last limit bytes of a stream.
type Tail struct {
	bytes []byte
	limit int
}

// NewTail creates a tail with the supplied byte limit.
func NewTail(limit int) *Tail { return &Tail{limit: max(0, limit)} }

// Push retains the newest bytes within the limit.
func (t *Tail) Push(chunk []byte) {
	if len(chunk) > t.limit {
		chunk = chunk[len(chunk)-t.limit:]
	}
	excess := max(0, len(t.bytes)+len(chunk)-t.limit)
	copy(t.bytes, t.bytes[excess:])
	t.bytes = append(t.bytes[:len(t.bytes)-excess], chunk...)
}

// Bytes returns the retained bytes, transferring their ownership to the caller.
func (t *Tail) Bytes() []byte {
	b := t.bytes
	t.bytes = nil
	return b
}

// Text returns the bytes as text, replacing invalid or cut UTF-8 characters.
func (t *Tail) Text() string { return streamText(t.bytes) }
