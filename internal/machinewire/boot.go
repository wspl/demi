package machinewire

import "fmt"

// Format keeps the boot credential out of diagnostics.
func (p WakeParams) Format(state fmt.State, _ rune) {
	// fmt.State writes into fmt's own buffer and cannot fail.
	_, _ = fmt.Fprintf(
		state,
		"WakeParams{DeviceID:%q BackendURL:%s DeviceToken:%v}",
		p.DeviceID,
		p.Boot.BackendURL,
		p.Boot.DeviceToken,
	)
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
