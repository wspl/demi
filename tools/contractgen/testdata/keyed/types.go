package keyed

import (
	"errors"
	blocks "github.com/wspl/demi/tools/contractgen/testdata/blocks"
)

//go:generate go run ../..

// +demi:id
// +demi:pattern ^key_[a-z]+$
// +demi:check permitted
type Key string

func permitted(k Key) error {
	if k == "key_reserved" {
		return errors.New("reserved key")
	}
	return nil
}

// +demi:root direction=receive output=plugin-keyed
// +demi:msgpack
type Record struct {
	Values map[Key]*string         `json:"values"`
	IDs    map[blocks.BlockId]bool `json:"ids"`
}

// +demi:root
// +demi:msgpack
type NamedMap map[Key]bool
