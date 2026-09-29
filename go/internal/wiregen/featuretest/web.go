package featuretest

import (
	"encoding/json/jsontext"
	"errors"
	"github.com/wspl/demi/go/internal/wire"
	"github.com/wspl/demi/go/internal/wiregen/exporttest"
	"strings"
)

//demi:wire
//demi:export
//demi:msgpack
type Embedded struct {
	exporttest.Interval
	Label Trimmed `json:"label" check:"chars=1..3"`
}

//demi:value
//demi:check nonul
//demi:decode
//demi:export
//demi:jsonschema inline {"type":"string","format":"trimmed"}
type Trimmed string

func (v *Trimmed) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	text, err := wire.ReadString(dec)
	if err != nil {
		return err
	}
	*v = Trimmed(strings.TrimSpace(text))
	return nil
}
func (v Trimmed) validate() error {
	if strings.TrimSpace(string(v)) != string(v) {
		return errors.New("must be trimmed")
	}
	return nil
}

//demi:wire
//demi:export
//demi:msgpack
type Containers struct {
	Map  exporttest.Intervals    `json:"map" check:"keys(chars=1..),each(func=exporttest.ValidateInterval)"`
	List exporttest.IntervalList `json:"list" check:"items=1..,each(func=exporttest.ValidateInterval)"`
}

//demi:opaque string chars=1..254 format=email
//demi:jsonschema inline {"type":"string","maxLength":254}
type Email struct{}

//demi:opaque number range=0..100
//demi:jsonschema named {"type":"number","minimum":0,"maximum":100}
type Percent struct{}

//demi:opaque
//demi:representation Values
type Configured struct {
	Values []Usage `json:"values" check:"items=1..1000"`
}

//demi:opaque
//demi:jsonschema ref Email
type EmailAlias struct{}
