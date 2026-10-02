package remotehost

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import "github.com/wspl/demi/internal/host"

// DecodeOutput reads kept-output records in the runner wire's encoding.
func DecodeOutput(bytes []byte, missing *host.Missing) (host.WholeOutput, error) {
	panic("not written: b-remotehost")
}

// EncodeOutput writes whole output as the runner wire's kept-output records.
func EncodeOutput(output host.WholeOutput) ([]byte, error) { panic("not written: b-remotehost") }
