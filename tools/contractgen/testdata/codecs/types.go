// Package codecs exercises delegation to contracts with private representations.
package codecs

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerproto"
)

//go:generate go run ../..

// +demi:root
// +demi:msgpack
// +demi:tolerant
type Empty struct{}

// +demi:root
type Wake struct {
	Boot runnerproto.ManagedBoot `json:"boot"`
}

// +demi:root
type Address struct {
	URL runnerproto.BackendURL `json:"url"`
}

// +demi:root
// +demi:msgpack
type Envelope struct {
	Value Private `json:"value"`
}

// +demi:codec
type Private struct{ value string }

func (v Private) MarshalJSON() ([]byte, error) {
	return contract.EncodeJSON(v.value)
}

func (v *Private) UnmarshalJSON(data []byte) error {
	value, err := contract.Decode[string](data)
	if err != nil {
		return err
	}
	return v.assign(value)
}

func (v Private) MarshalMsgpack() ([]byte, error) {
	return contract.EncodeMsgpack(v.value)
}

func (v *Private) UnmarshalMsgpack(data []byte) error {
	value, err := contract.DecodeMsgpack[string](data)
	if err != nil {
		return err
	}
	return v.assign(value)
}

// assign models a codec that rejects and normalizes its own wire values.
func (v *Private) assign(value string) error {
	if value == "" {
		return fmt.Errorf("empty private value")
	}
	v.value = strings.ToLower(value)
	return nil
}
