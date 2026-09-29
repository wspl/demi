//go:build !testing

package edge

// TestControl is the hooks of a test build (control.go), which a release
// build does not have.
type TestControl struct{}

// ControlFromEnvironment is nil in a release build: it reads no tuning file
// and serves no control socket.
func ControlFromEnvironment(*Config) (*TestControl, error) { return nil, nil }

// Serve, Stop and End are never called on the nil control.
func (*TestControl) Serve(*Server) error { return nil }
func (*TestControl) Stop()               {}
func (*TestControl) End()                {}
