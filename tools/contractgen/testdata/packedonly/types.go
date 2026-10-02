package packedonly

//go:generate go run ../.. .

// +demi:msgpack
type Value struct {
	Text string `json:"text"`
}
