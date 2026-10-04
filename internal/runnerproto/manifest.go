package runnerproto

import (
	"encoding/json"
	"fmt"
	"slices"

	"github.com/wspl/demi/internal/commanddecl"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/contract"
)

// +demi:root
// +demi:check verifyManifest
type Manifest struct {
	// The SHA-256 of the canonical JSON of `roots` and `packages`.
	Hash  string          `json:"hash"`
	Roots map[string]Root `json:"roots"`
	// Each package descriptor, under its digest.
	Packages map[string]commandproto.PackageDescriptor `json:"packages"`
}

// Root holds a command tree, decoded through declare when the manifest is verified.
type Root struct {
	Tree json.RawMessage `json:"tree"`
}

// BuildManifest files package descriptors under their digests, pins native
// declarations to them, validates the trees, and hashes the manifest.
func BuildManifest(
	roots []commanddecl.Node[commanddecl.NativeOperation],
	packages []commandproto.PackageDescriptor,
) (Manifest, error) {
	manifest := Manifest{Roots: make(map[string]Root), Packages: make(map[string]commandproto.PackageDescriptor)}
	digests := make(map[string]string)
	for _, descriptor := range packages {
		digest, err := descriptor.Digest()
		if err != nil {
			return Manifest{}, fmt.Errorf("package digest: %w", err)
		}
		if _, exists := digests[descriptor.ID]; exists {
			return Manifest{}, fmt.Errorf("duplicate command package: %s", descriptor.ID)
		}
		digests[descriptor.ID] = digest
		manifest.Packages[digest] = descriptor
	}
	for _, root := range roots {
		tree, err := commanddecl.Pin(root, func(operation commanddecl.NativeOperation) (string, error) {
			digest, ok := digests[operation.Package]
			if !ok {
				return "", fmt.Errorf("command package is not configured: %s", operation.Package)
			}
			if !slices.Contains(manifest.Packages[digest].Operations, operation.Operation) {
				return "", fmt.Errorf("command package %s has no operation %s", operation.Package, operation.Operation)
			}
			return digest, nil
		})
		if err != nil {
			return Manifest{}, fmt.Errorf("pin root: %w", err)
		}
		if err := tree.Validate(); err != nil {
			return Manifest{}, fmt.Errorf("validate root: %w", err)
		}
		name := commanddecl.Name(tree)
		if _, exists := manifest.Roots[name]; exists {
			return Manifest{}, fmt.Errorf("duplicate root command: %s", name)
		}
		data, err := contract.EncodeJSON(tree)
		if err != nil {
			return Manifest{}, fmt.Errorf("encode tree: %w", err)
		}
		manifest.Roots[name] = Root{Tree: data}
	}
	hash, err := manifestHash(manifest)
	if err != nil {
		return Manifest{}, err
	}
	manifest.Hash = hash
	return manifest, nil
}

// verifyManifest checks package identity, declaration rules, native bindings,
// root names, and the digest covering both catalogs.
func verifyManifest(manifest Manifest) error {
	ids := make(map[string]bool)
	for digest, descriptor := range manifest.Packages {
		actual, err := descriptor.Digest()
		if err != nil {
			return fmt.Errorf("package digest: %w", err)
		}
		if ids[descriptor.ID] || actual != digest {
			return fmt.Errorf("duplicate or corrupt command package: %s", descriptor.ID)
		}
		ids[descriptor.ID] = true
	}
	for name, root := range manifest.Roots {
		tree, err := commanddecl.DecodeManifestNode(root.Tree)
		if err != nil {
			return fmt.Errorf("root %s: %w", name, err)
		}
		if name != commanddecl.Name(tree) {
			return fmt.Errorf("manifest root name mismatch")
		}
		if err := tree.Validate(); err != nil {
			return fmt.Errorf("root %s: %w", name, err)
		}
		for _, leaf := range tree.Leaves() {
			binding, native := leaf.Binding()
			if !native {
				continue
			}
			descriptor, ok := manifest.Packages[binding.DescriptorHash]
			if !ok || descriptor.ID != binding.Package || !slices.Contains(descriptor.Operations, binding.Operation) {
				return fmt.Errorf("unresolved native command binding")
			}
		}
	}
	actual, err := manifestHash(manifest)
	if err != nil {
		return err
	}
	if actual != manifest.Hash {
		return fmt.Errorf("command manifest hash mismatch")
	}
	return nil
}

// manifestHash excludes the claimed hash from the canonical digest.
func manifestHash(manifest Manifest) (string, error) {
	return commandproto.CanonicalDigest(map[string]any{"roots": manifest.Roots, "packages": manifest.Packages})
}
