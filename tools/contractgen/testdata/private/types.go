// Package private exercises generated entry points for unexported contracts.
package private

//go:generate go run ../..

// +demi:root
// +demi:id
type issuer string

// +demi:root
// +demi:msgpack
// +demi:union tag=kind
type rawBinding interface{ binding() }

// +demi:variant rawBinding leaf
type rawLeaf struct {
	Issuer issuer `json:"issuer"`
}

// RoundTrip uses the package-private generated API in ordinary package code.
func RoundTrip(data []byte) ([]byte, error) {
	value, err := decodeRawBinding(data)
	if err != nil {
		return nil, err
	}
	if err := validateRawBinding(value); err != nil {
		return nil, err
	}
	if _, err := parseIssuer("issuer"); err != nil {
		return nil, err
	}
	if _, err := decodeIssuer([]byte(`"issuer"`)); err != nil {
		return nil, err
	}
	packed, err := encodeRawBindingMsgpack(value)
	if err != nil {
		return nil, err
	}
	value, err = decodeRawBindingMsgpack(packed)
	if err != nil {
		return nil, err
	}
	return (rawBindingJSON{Value: value}).MarshalJSON()
}
