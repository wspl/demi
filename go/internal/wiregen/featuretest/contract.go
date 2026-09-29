// Package featuretest exercises declarations used by backend and vendor contracts.
package featuretest

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import (
	"encoding/json/jsontext"
	"errors"
	"github.com/wspl/demi/go/builtinproto"
	"github.com/wspl/demi/go/internal/wiregen/exporttest"
)

//demi:wire
//demi:export
//demi:msgpack
type Patch struct {
	Usage    *Usage   `json:"usage,omitzero" check:"nullabsent"`
	Setting  **string `json:"setting,omitzero" check:"nullable,chars=1.."`
	Forkable bool     `json:"forkable,omitzero"`
}

//demi:wire
type Usage struct {
	Tokens uint64 `json:"tokens" check:"range=1.."`
}

//demi:wire
//demi:export
//demi:msgpack
type Reasoning struct {
	ID     string                    `json:"id" check:"chars=1.."`
	Hidden bool                      `json:"hidden,omitzero"`
	Extra  map[string]jsontext.Value `json:",inline"`
}

//demi:wire
type Bounds struct {
	Low  int64 `json:"low" check:"range=0.."`
	High int64 `json:"high"`
}

func (v Bounds) check() error {
	if v.Low > v.High {
		return errors.New("low exceeds high")
	}
	return nil
}

//demi:wire
type BoundInfo struct{ Bounds }

//demi:union tag=type
//demi:export
//demi:msgpack
type Event interface{ event() }

//demi:variant changed
type Changed struct{ BoundInfo }

func (Changed) event() {}

//demi:wire
//demi:export
type Foreign struct {
	Mode   exporttest.Mode         `json:"mode" check:"func=exporttest.ValidateMode"`
	Choice exporttest.Choice       `json:"choice" check:"func=exporttest.ValidateChoice"`
	Tab    builtinproto.BrowserTab `json:"tab" check:"func=builtinproto.ValidateBrowserTab"`
}

//demi:opaque string format=date-time
type Timestamp struct{ Text string }

func (v *Timestamp) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	token, err := dec.ReadToken()
	if err != nil {
		return err
	}
	if token.Kind() != '"' {
		return errors.New("expected timestamp string")
	}
	v.Text = token.String()
	return nil
}
func (v Timestamp) MarshalJSONTo(enc *jsontext.Encoder) error {
	return enc.WriteToken(jsontext.String(v.Text))
}

//demi:wire
//demi:export
type Timed struct {
	At Timestamp `json:"at"`
}

//demi:union tag=kind content=data
//demi:export
//demi:msgpack
type Replay interface{ replay() }

//demi:variant reasoning
type ReplayReasoning struct {
	ID    string                    `json:"id" check:"chars=1.."`
	Extra map[string]jsontext.Value `json:",inline"`
}

func (ReplayReasoning) replay() {}

//demi:wire
//demi:export
type Envelope struct {
	Replay   Replay `json:",inline"`
	Forkable bool   `json:"forkable,omitzero"`
}
