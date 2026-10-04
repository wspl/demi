// Copyright (c) 2026, the Demi contributors
// See LICENSE for licensing information

package interp

import (
	"os"
	"sync/atomic"
)

// procSubstPath keeps a FIFO named until both its receiving statement and its
// substitution task finish. Neither owner waits for the other during cleanup:
// the receiving statement may itself run the wait builtin.
type procSubstPath struct {
	path      string
	remaining atomic.Int32
}

func (p *procSubstPath) release() error {
	if p.remaining.Add(-1) != 0 {
		return nil
	}
	if err := os.Remove(p.path); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
