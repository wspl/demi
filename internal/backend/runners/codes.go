package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import "github.com/wspl/demi/internal/runnerwire"

// ClaimCode is a pairing code: its bits, however its user spells them.
type ClaimCode [16]byte

// GenerateClaimCode makes a code from 128 random bits.
func GenerateClaimCode() ClaimCode { panic("not written: b-runners") }

// ParseClaimCode accepts 26 alphabet characters in either case, with dashes and
// spaces, reading O as 0 and I and L as 1. Invalid text returns false.
func ParseClaimCode(text string) (ClaimCode, bool) { panic("not written: b-runners") }

// Printed returns the code as the runner prints it, in groups of four.
func (c ClaimCode) Printed() string { panic("not written: b-runners") }

// NewDeviceToken makes a new device's credential from 256 random bits.
func NewDeviceToken() runnerwire.DeviceToken { panic("not written: b-runners") }
