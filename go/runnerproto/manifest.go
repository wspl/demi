package runnerproto

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"maps"
	"slices"

	"github.com/wspl/demi/go/commandservice"
	"github.com/wspl/demi/go/commandtree"
	"github.com/wspl/demi/go/internal/wire"
)

// Manifest identifies the command trees and the native packages they bind to.
//
//demi:wire
type Manifest struct {
	Hash     string                                      `json:"hash"`
	Roots    map[string]Root                             `json:"roots"`
	Packages map[string]commandservice.PackageDescriptor `json:"packages" check:"each(func=commandservice.Validate)"`
}

// Root delegates the declaration tree's boundary to commandtree.
//
//demi:opaque
type Root struct {
	Tree commandtree.Node `json:"tree"`
}

// UnmarshalJSONFrom reads the root's envelope and delegates its tree to its owner.
func (r *Root) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	*r = Root{}
	if err := wire.BeginObject(dec); err != nil {
		return err
	}
	found := false
	for {
		name, more, err := wire.NextMember(dec)
		if err != nil {
			return err
		}
		if !more {
			break
		}
		if name != "tree" {
			return wire.Unknown(name)
		}
		raw, err := dec.ReadValue()
		if err != nil {
			return wire.In(name, err)
		}
		r.Tree, err = commandtree.DecodeNode(raw)
		if err != nil {
			return wire.In(name, err)
		}
		found = true
	}
	if !found {
		return wire.Required("tree")
	}
	return nil
}

// MarshalJSONTo uses commandtree's union codec, including its package options.
func (r Root) MarshalJSONTo(enc *jsontext.Encoder) error {
	tree, err := commandtree.EncodeNode(r.Tree)
	if err != nil {
		return err
	}
	return json.MarshalEncode(enc, struct {
		Tree jsontext.Value `json:"tree"`
	}{tree})
}

// DecodeManifest verifies descriptors, command bindings and the canonical hash.
func DecodeManifest(data []byte) (Manifest, error) { return decode[Manifest](data) }

// Encode returns the verified manifest as JSON.
func (m Manifest) Encode() ([]byte, error) { return encode(m) }

func (m Manifest) check() error {
	ids := map[string]bool{}
	for _, digest := range slices.Sorted(maps.Keys(m.Packages)) {
		descriptor := m.Packages[digest]
		actual, err := descriptor.Digest()
		if err != nil {
			return err
		}
		if ids[descriptor.ID] || actual != digest {
			return fmt.Errorf("duplicate or corrupt native package: %s", descriptor.ID)
		}
		ids[descriptor.ID] = true
	}
	for _, name := range slices.Sorted(maps.Keys(m.Roots)) {
		root := m.Roots[name]
		if name != commandtree.Name(root.Tree) {
			return errors.New("manifest root name mismatch")
		}
		if err := commandtree.Validate(root.Tree); err != nil {
			return err
		}
		for _, leaf := range commandtree.Leaves(root.Tree) {
			binding := leaf.Binding
			if binding == nil {
				continue
			}
			descriptor, ok := m.Packages[binding.DescriptorHash]
			if !ok || descriptor.ID != binding.Package || !slices.Contains(descriptor.Operations, binding.Operation) {
				return errors.New("unresolved native command binding")
			}
		}
	}
	hash, err := m.canonicalHash()
	if err != nil {
		return err
	}
	if hash != m.Hash {
		return errors.New("command manifest hash mismatch")
	}
	return nil
}

func (m Manifest) canonicalHash() (string, error) {
	data, err := json.Marshal(struct {
		Roots    map[string]Root                             `json:"roots"`
		Packages map[string]commandservice.PackageDescriptor `json:"packages"`
	}{m.Roots, m.Packages}, json.Deterministic(true))
	if err != nil {
		return "", err
	}
	canonical := jsontext.Value(data)
	if err := canonical.Canonicalize(); err != nil {
		return "", err
	}
	sum := sha256.Sum256(canonical)
	return hex.EncodeToString(sum[:]), nil
}

// BuildManifest pins each native declaration to its package descriptor, checks
// the resulting trees, and computes their canonical identity.
func BuildManifest(roots []commandtree.Node, packages []commandservice.PackageDescriptor) (Manifest, error) {
	m := Manifest{Roots: map[string]Root{}, Packages: map[string]commandservice.PackageDescriptor{}}
	digests := map[string]string{}
	for _, descriptor := range packages {
		digest, err := descriptor.Digest()
		if err != nil {
			return Manifest{}, err
		}
		if _, ok := digests[descriptor.ID]; ok {
			return Manifest{}, fmt.Errorf("duplicate native package: %s", descriptor.ID)
		}
		digests[descriptor.ID] = digest
		m.Packages[digest] = descriptor
	}
	for _, root := range roots {
		tree, err := commandtree.Pin(root, func(operation commandtree.NativeOperation) (string, error) {
			digest, ok := digests[operation.Package]
			if !ok {
				return "", fmt.Errorf("native package is not configured: %s", operation.Package)
			}
			if !slices.Contains(m.Packages[digest].Operations, operation.Operation) {
				return "", fmt.Errorf("native package %s has no operation %s", operation.Package, operation.Operation)
			}
			return digest, nil
		})
		if err != nil {
			return Manifest{}, err
		}
		if err := commandtree.Validate(tree); err != nil {
			return Manifest{}, err
		}
		name := commandtree.Name(tree)
		if _, ok := m.Roots[name]; ok {
			return Manifest{}, fmt.Errorf("duplicate root command: %s", name)
		}
		m.Roots[name] = Root{tree}
	}
	var err error
	m.Hash, err = m.canonicalHash()
	return m, err
}
