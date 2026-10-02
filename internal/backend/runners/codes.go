package runners

import (
	"crypto/rand"
	"encoding/base32"
	"encoding/hex"
	"strings"

	"github.com/wspl/demi/internal/runnerwire"
)

var crockford = base32.NewEncoding("0123456789ABCDEFGHJKMNPQRSTVWXYZ").WithPadding(base32.NoPadding)

// ClaimCode is a pairing code: its bits, however its user spells them.
type ClaimCode [16]byte

// GenerateClaimCode makes a code from 128 random bits.
func GenerateClaimCode() ClaimCode {
	var code ClaimCode
	rand.Read(code[:])
	return code
}

// ParseClaimCode accepts 26 alphabet characters in either case, with dashes and
// spaces, reading O as 0 and I and L as 1. Invalid text returns false.
func ParseClaimCode(text string) (ClaimCode, bool) {
	normalized := strings.Map(func(r rune) rune {
		switch r {
		case ' ', '\t', '\r', '\n', '-':
			return -1
		case 'o', 'O':
			return '0'
		case 'i', 'I', 'l', 'L':
			return '1'
		}
		if r >= 'a' && r <= 'z' {
			return r - 'a' + 'A'
		}
		return r
	}, text)
	bytes, err := crockford.DecodeString(normalized)
	// The standard decoder ignores unused trailing bits; Crockford refuses them.
	if err != nil || len(bytes) != 16 || crockford.EncodeToString(bytes) != normalized {
		return ClaimCode{}, false
	}
	return ClaimCode(bytes), true
}

// Printed returns the code as the runner prints it, in groups of four.
func (c ClaimCode) Printed() string {
	text := crockford.EncodeToString(c[:])
	var groups []string
	for len(text) > 4 {
		groups = append(groups, text[:4])
		text = text[4:]
	}
	return strings.Join(append(groups, text), "-")
}

// NewDeviceToken makes a new device's credential from 256 random bits.
func NewDeviceToken() runnerwire.DeviceToken {
	var bits [32]byte
	rand.Read(bits[:])
	// Every 64-digit lowercase hexadecimal string satisfies DeviceToken's contract.
	token, _ := runnerwire.ParseDeviceToken(hex.EncodeToString(bits[:]))
	return token
}
