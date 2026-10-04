package runners

import (
	"context"
	"fmt"
	"maps"
	"slices"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/commandproto"
)

// Store is where runners download loaded packages' executables.
//
//sumtype:decl
type Store interface{ nativeStore() }

// UnpublishedStore has no packages and therefore nothing to download.
type UnpublishedStore struct{}

func (*UnpublishedStore) nativeStore() {}
func (*SignedArtifacts) nativeStore()  {}
func (*LocalArtifacts) nativeStore()   {}

// NativeCatalog holds loaded command packages and their artifact store. Its
// descriptors are immutable; it can be shared by shards. Construct it with
// NewNativeCatalog, UnpublishedCatalog or PublishNative.
type NativeCatalog struct {
	packages   []commandproto.PackageDescriptor
	store      Store
	closeOnce  sync.Once
	closeStore func() error
	closeErr   error
}

// NewNativeCatalog builds the catalog, refusing invalid descriptors or duplicate
// package IDs. It borrows store; its external owner retains store cleanup.
func NewNativeCatalog(packages []commandproto.PackageDescriptor, store Store) (*NativeCatalog, error) {
	if _, err := remotehost.NewCommandCatalog(packages, unpublished{}); err != nil {
		return nil, err
	}
	owned := make([]commandproto.PackageDescriptor, 0, len(packages))
	for _, descriptor := range packages {
		owned = append(owned, cloneDescriptor(descriptor))
	}
	return &NativeCatalog{packages: owned, store: store}, nil
}

// UnpublishedCatalog makes an empty catalog. Commands declaring a native command
// select no manifest; product startup instead loads releases with PublishNative.
func UnpublishedCatalog() *NativeCatalog {
	return &NativeCatalog{store: &UnpublishedStore{}}
}

// Catalog builds a command catalog, locating development downloads on backend.
func (c *NativeCatalog) Catalog(backend *PublicURL) *remotehost.CommandCatalog {
	// Descriptors were validated and copied at construction and never mutate.
	catalog, _ := remotehost.NewCommandCatalog(c.packages, c.Resolver(backend))
	return catalog
}

// Resolver locates executables for work that binds a package without a manifest,
// such as a user stream or one-shot call. Development downloads use backend.
func (c *NativeCatalog) Resolver(backend *PublicURL) remotehost.ArtifactResolver {
	switch store := c.store.(type) {
	case *UnpublishedStore:
		return unpublished{}
	case *SignedArtifacts:
		return store
	case *LocalArtifacts:
		return &ServedArtifacts{Artifacts: store, Backend: backend}
	}
	return unpublished{}
}

// LocalArtifact returns a development download, and false for a nonlocal store or an unknown sha256.
func (c *NativeCatalog) LocalArtifact(ctx context.Context, sha256 string) (LocalArtifact, bool, error) {
	if store, ok := c.store.(*LocalArtifacts); ok {
		return store.Artifact(ctx, sha256)
	}
	return nil, false, nil
}

// Package returns an owned copy of the loaded descriptor by ID, and false when none has it.
func (c *NativeCatalog) Package(id string) (commandproto.PackageDescriptor, bool) {
	for _, descriptor := range c.packages {
		if descriptor.ID == id {
			owned := cloneDescriptor(descriptor)
			return owned, true
		}
	}
	return commandproto.PackageDescriptor{}, false
}

// Serves reports whether the loaded package serves every named operation.
func (c *NativeCatalog) Serves(packageID string, operations []string) bool {
	for _, descriptor := range c.packages {
		if descriptor.ID != packageID {
			continue
		}
		for _, operation := range operations {
			if !slices.Contains(descriptor.Operations, operation) {
				return false
			}
		}
		return true
	}
	return false
}

// Close releases resources opened by PublishNative after all catalog users have
// stopped. It is idempotent; a catalog borrowing an external store closes nothing.
func (c *NativeCatalog) Close(_ context.Context) error {
	c.closeOnce.Do(func() {
		if c.closeStore != nil {
			c.closeErr = c.closeStore()
		}
	})
	return c.closeErr
}

// unpublished refuses artifact lookups on a catalog containing no package.
type unpublished struct{}

// Resolve locates an executable for the requesting job.
func (unpublished) Resolve(
	_ context.Context,
	_ commandproto.PackageArtifact,
	target string,
) (commandproto.ArtifactLocation, error) {
	return nil, fmt.Errorf("no command package is published for %s", target)
}

// cloneDescriptor gives each catalog caller its own package collections.
func cloneDescriptor(value commandproto.PackageDescriptor) commandproto.PackageDescriptor {
	value.Operations = slices.Clone(value.Operations)
	value.Targets = maps.Clone(value.Targets)
	value.Resources = maps.Clone(value.Resources)
	for key, resource := range value.Resources {
		resource.Targets = maps.Clone(resource.Targets)
		value.Resources[key] = resource
	}
	return value
}
