package remotehosttest

//revive:disable:unused-parameter
// API checkpoint: keep parameter names for callers until the bodies are implemented.

import (
	"context"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
)

// NativeFixture is a built native command package and its executable location.
type NativeFixture struct {
	Descriptor commandwire.PackageDescriptor
}

// LoadNativeFixture resolves and describes the runner's native test package.
func LoadNativeFixture(ctx context.Context) (*NativeFixture, error) {
	panic("not written: b-remotehost")
}

// NewNativeFixture describes the package served by path for this machine's target.
func NewNativeFixture(ctx context.Context, id, path string, operations []string) (*NativeFixture, error) {
	panic("not written: b-remotehost")
}

// Resolver resolves the package executable to its path on this machine.
func (f *NativeFixture) Resolver() remotehost.ArtifactResolver { panic("not written: b-remotehost") }
