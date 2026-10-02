package transcript

import (
	"crypto/rand"
	"encoding/hex"
)

// IDs supplies nonempty identities, each different from every other it supplied.
// The session serializes calls to its identity source.
type IDs interface {
	// NextID returns the next block, turn, request, or transcript epoch identity.
	NextID() string
}

// RandomIDs supplies version 4 UUIDs as 32 hexadecimal digits.
type RandomIDs struct{}

// NextID returns a fresh random identity.
func (RandomIDs) NextID() string {
	var id [16]byte
	// crypto/rand.Read fills the buffer or terminates the process; it never returns an error.
	_, _ = rand.Read(id[:])
	id[6] = (id[6] & 0x0f) | 0x40
	id[8] = (id[8] & 0x3f) | 0x80
	return hex.EncodeToString(id[:])
}
