package runners

//revive:disable:unused-parameter
// API checkpoint: parameters are consumed by the implementation checkpoint.

import (
	"context"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandwire"
)

// Store is where runners download loaded packages' executables.
//
//sumtype:decl
type Store interface{ nativeStore() }

// UnpublishedStore has no packages and therefore nothing to download.
type UnpublishedStore struct{}

func (*UnpublishedStore) nativeStore() { panic("not written: b-runners") }
func (*SignedArtifacts) nativeStore()  { panic("not written: b-runners") }
func (*LocalArtifacts) nativeStore()   { panic("not written: b-runners") }

// NativeCatalog holds loaded command packages and their artifact store. Its
// descriptors are immutable; it can be shared by shards. Construct it with
// NewNativeCatalog, UnpublishedCatalog or PublishNative.
type NativeCatalog struct{}

// NewNativeCatalog builds the catalog, refusing invalid descriptors or duplicate
// package IDs. It borrows store; its external owner retains store cleanup.
func NewNativeCatalog(packages []commandwire.PackageDescriptor, store Store) (*NativeCatalog, error) {
	panic("not written: b-runners")
}

// UnpublishedCatalog makes an empty catalog. Commands declaring a native command
// select no manifest; product startup instead loads releases with PublishNative.
func UnpublishedCatalog() *NativeCatalog { panic("not written: b-runners") }

// Catalog builds a command catalog, locating development downloads on backend.
func (c *NativeCatalog) Catalog(backend *PublicURL) *remotehost.CommandCatalog {
	panic("not written: b-runners")
}

// Resolver locates executables for work that binds a package without a manifest,
// such as a user stream or one-shot call. Development downloads use backend.
func (c *NativeCatalog) Resolver(backend *PublicURL) remotehost.ArtifactResolver {
	panic("not written: b-runners")
}

// LocalArtifact returns a development download, or nil for a nonlocal store or an
// unknown sha256. An error means a known artifact could not be read or encoded.
func (c *NativeCatalog) LocalArtifact(ctx context.Context, sha256 string) (LocalArtifact, error) {
	panic("not written: b-runners")
}

// Package returns the loaded descriptor by ID, or nil; the result is an owned copy.
func (c *NativeCatalog) Package(id string) *commandwire.PackageDescriptor {
	panic("not written: b-runners")
}

// Serves reports whether the loaded package serves every named operation.
func (c *NativeCatalog) Serves(packageID string, operations []string) bool {
	panic("not written: b-runners")
}

// Close releases resources opened by PublishNative after all catalog users have
// stopped. It is idempotent; a catalog borrowing an external store closes nothing.
func (c *NativeCatalog) Close(ctx context.Context) error { panic("not written: b-runners") }
