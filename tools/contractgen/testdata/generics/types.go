// Package generics exercises declaration loading beside generic behavior.
package generics

//go:generate go run ../..

// +demi:root
type Message struct {
	Text string `json:"text"`
}

func Identity[T any](value T) T {
	return value
}

type Box[T any] struct {
	Value T
}

func (b Box[T]) Unwrap() T {
	return b.Value
}

func (b *Box[T]) Set(value T) {
	b.Value = value
}

type Pair[A, B any] struct {
	First  A
	Second B
}

func (p Pair[A, B]) Left() A {
	return p.First
}

func (p *Pair[A, B]) Right() B {
	return p.Second
}

// Decode relies on generated output that is absent during declaration loading.
func Decode(data []byte) (Message, error) {
	return DecodeMessage(data)
}
