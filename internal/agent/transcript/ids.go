package transcript

// IDs supplies nonempty identities, each different from every other it supplied.
// The session serializes calls to its identity source.
type IDs interface {
	// NextID returns the next block, turn, request, or transcript epoch identity.
	NextID() string
}

// RandomIDs supplies version 4 UUIDs as 32 hexadecimal digits.
type RandomIDs struct{}

// NextID returns a fresh random identity.
func (RandomIDs) NextID() string { panic("not written: a-transcript") }
