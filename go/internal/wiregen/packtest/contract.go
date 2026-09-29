// Package packtest is the executable fixture for generated MessagePack contracts.
package packtest

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

import "encoding/json/jsontext"

//demi:wire
//demi:msgpack
type Record struct {
	Ratio    float64            `json:"ratio"`
	Name     string             `json:"name" check:"chars=1..,nonul"`
	Small    uint8              `json:"small"`
	Optional *string            `json:"optional,omitzero"`
	Nullable *int32             `json:"nullable" check:"nullable"`
	Data     []uint8            `json:"data" msgpack:"bin"`
	At       int64              `json:"at" msgpack:"timestamp"`
	Items    []Item             `json:"items"`
	Labels   map[string]*string `json:"labels"`
	Raw      jsontext.Value     `json:"raw"`
	Event    Event              `json:"event"`
	Choice   Choice             `json:"choice"`
	Call     Call               `json:"call"`
}

//demi:wire open
type Item struct {
	Kind string `json:"kind" check:"oneof=yes|no"`
}

//demi:union tag=type
type Event interface{ event() }

//demi:variant changed
type Changed struct {
	Count int16 `json:"count" check:"range=1..9"`
}

func (Changed) event() {}

//demi:variant empty
type Empty struct{}

func (Empty) event() {}

//demi:union untagged
type Choice interface{ choice() }

//demi:variant
type Text string

func (Text) choice() {}

//demi:variant
type Named struct {
	Name string `json:"name"`
}

func (Named) choice() {}

//demi:union tag=op content=params
type Call interface{ call() }

//demi:variant run open
type Run struct {
	Limit uint16 `json:"limit" check:"range=1.."`
}

func (Run) call() {}

//demi:wire
//demi:msgpack
type Envelope struct {
	ID   string `json:"id"`
	Call Call   `json:",inline"`
}
