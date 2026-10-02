package machinewire

import (
	"fmt"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerwire"
)

// validateWakeParams delegates the opaque boot record to its owning boundary.
func validateWakeParams(p WakeParams) error {
	_, err := runnerwire.DecodeManagedBoot(p.Boot)
	if err != nil {
		return contract.At("boot", err)
	}
	return nil
}

// ManagedBoot reads the checked boot record through its owning decoder.
func (p WakeParams) ManagedBoot() (runnerwire.ManagedBoot, error) {
	return runnerwire.DecodeManagedBoot(p.Boot)
}

// Format keeps the boot credential out of diagnostics.
func (p WakeParams) Format(state fmt.State, _ rune) {
	boot, err := p.ManagedBoot()
	if err != nil {
		// fmt.State writes to fmt's buffer and cannot fail. Invalid raw data can contain a credential.
		_, _ = fmt.Fprint(state, "WakeParams{invalid boot}")
		return
	}
	_, _ = fmt.Fprintf(state, "WakeParams{DeviceID:%q BackendURL:%s DeviceToken:DeviceToken(..)}", p.DeviceID, boot.BackendURL)
}

// Format carries the redacted boot diagnostic through a request's call interface.
func (w *Wake) Format(state fmt.State, verb rune) {
	if w == nil {
		// fmt.State writes into fmt's own buffer and cannot fail.
		_, _ = fmt.Fprint(state, "<nil>")
		return
	}
	w.Params.Format(state, verb)
}
