//revive:disable:unused-parameter API checkpoint retains parameter names for callers; bodies follow after merge.

package cmdpkgs

import (
	"context"

	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ServiceRegistry owns resident services and their lifecycle work. It must be closed.
type ServiceRegistry struct{}

// NewServiceRegistry starts services in cwd with exactly env, caching artifacts in
// cache and consulting image first when nonempty. The caller must Close the registry.
func NewServiceRegistry(ctx context.Context, cache, image, cwd string, env map[string]string) (*ServiceRegistry, error) {
	panic("not written: r-cmdpkgs")
}

// Handle returns a handle sharing this registry.
func (r *ServiceRegistry) Handle() *ServiceHandle { panic("not written: r-cmdpkgs") }

// Installs observes the installs made by service starts and artifact requests.
func (r *ServiceRegistry) Installs() *InstallsReceiver { panic("not written: r-cmdpkgs") }

// Close stops every service and joins the registry's work. If ctx ends first,
// shutdown continues and a later Close can wait for it. Close is idempotent.
func (r *ServiceRegistry) Close(ctx context.Context) error { panic("not written: r-cmdpkgs") }

// ServiceHandle sends requests to one shared registry; it is safe for concurrent use.
type ServiceHandle struct{}

// Invoking registers invocation in pkg against its work's artifact resolver.
// The caller must release the returned registration when the invocation ends.
func (h *ServiceHandle) Invoking(invocation, pkg string, resolver ArtifactResolver) *Invoking {
	panic("not written: r-cmdpkgs")
}

// Lease keeps digest's service resident until Release, even before it starts.
func (h *ServiceHandle) Lease(ctx context.Context, digest string) (*ServiceLease, error) {
	panic("not written: r-cmdpkgs")
}

// Acquire returns descriptor's service for this Host. Concurrent callers share
// one start, while cancellation abandons only this caller's wait.
func (h *ServiceHandle) Acquire(ctx context.Context, descriptor commandwire.PackageDescriptor, resolver ArtifactResolver, numbers NumberSource) (*Resident, error) {
	panic("not written: r-cmdpkgs")
}

// ReleaseConversation releases a conversation in all running services concurrently
// and retires each service whose release fails.
func (h *ServiceHandle) ReleaseConversation(ctx context.Context, conversation string) error {
	panic("not written: r-cmdpkgs")
}

// StopAll stops every service and waits until each has ended, as connection loss does.
func (h *ServiceHandle) StopAll(ctx context.Context) error { panic("not written: r-cmdpkgs") }

// ServiceLease claims one digest's service until explicitly released.
type ServiceLease struct{}

// Release ends the claim exactly once; repeated calls do nothing.
func (l *ServiceLease) Release() { panic("not written: r-cmdpkgs") }

// Invoking is a registered invocation whose release cancels its artifact waits.
type Invoking struct{}

// Release unregisters the invocation exactly once and cancels its artifact waits.
func (i *Invoking) Release() { panic("not written: r-cmdpkgs") }

// Resident is a running service a caller invokes. It does not replace a service lease.
type Resident struct{}

// Client returns the service's shared client. The registry owns and closes it.
func (r *Resident) Client() *cmdsdk.Client { panic("not written: r-cmdpkgs") }

// Failure resolves a transport failure to the service's exit and stderr tail when
// the connection was lost, waiting for its process to be reaped.
func (r *Resident) Failure(ctx context.Context, err error) error { panic("not written: r-cmdpkgs") }
