package provider

import (
	"crypto/rand"
	"fmt"
)

// NewToolUseID identifies a vendor tool call that supplied no ID.
func NewToolUseID(prefix string) string {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err != nil {
		panic(err)
	} // crypto/rand terminates the process if its source fails.
	bytes[6] = bytes[6]&15 | 64
	bytes[8] = bytes[8]&63 | 128
	return fmt.Sprintf("%s%x-%x-%x-%x-%x", prefix, bytes[:4], bytes[4:6], bytes[6:8], bytes[8:10], bytes[10:])
}
