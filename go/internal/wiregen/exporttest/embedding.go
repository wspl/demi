package exporttest

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"math"
)

//demi:wire
//demi:export
//demi:msgpack
type Interval struct {
	Low  int `json:"low" check:"range=1.."`
	High int `json:"high"`
}

func (v Interval) check() error {
	if v.Low > v.High {
		return errors.New("low exceeds high")
	}
	return nil
}

type Intervals = map[string]Interval
type IntervalList []Interval

// Rounded is an owner whose own encoding differs from its fields: it writes
// its value rounded to a whole number, in JSON and in MessagePack alike, so a
// struct that embeds it must write it through its owner's encoder.
//
//demi:wire
//demi:export
//demi:msgpack
type Rounded struct {
	Value float64 `json:"value"`
}

// MarshalJSONTo writes the value rounded, as normalizeWire does for MessagePack.
func (v Rounded) MarshalJSONTo(enc *jsontext.Encoder) error {
	rounded, err := v.normalizeWire()
	if err != nil {
		return err
	}
	return json.MarshalEncode(enc, struct {
		Value float64 `json:"value"`
	}{rounded.Value})
}

func (v Rounded) normalizeWire() (Rounded, error) {
	v.Value = math.Round(v.Value)
	return v, nil
}
