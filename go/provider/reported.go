package provider

import (
	"encoding/json/jsontext"
	"encoding/json/v2"
)

// ReportedNumber retains display-only numeric claims, treating another shape as absent.
//
//demi:opaque
type ReportedNumber struct{ Value *float64 }

func (v *ReportedNumber) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.Value = nil
	if raw.Kind() == '0' {
		var value float64
		if json.Unmarshal(raw, &value) == nil {
			v.Value = &value
		}
	}
	return nil
}
func (ReportedNumber) validate() error { return nil }

//demi:opaque
type ReportedBool struct{ Value *bool }

func (v *ReportedBool) UnmarshalJSONFrom(dec *jsontext.Decoder) error {
	raw, err := dec.ReadValue()
	if err != nil {
		return err
	}
	v.Value = nil
	if raw.Kind() == 't' || raw.Kind() == 'f' {
		value := raw.Kind() == 't'
		v.Value = &value
	}
	return nil
}
func (ReportedBool) validate() error { return nil }
