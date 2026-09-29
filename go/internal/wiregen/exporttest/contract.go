// Package exporttest owns contracts exported without opting into MessagePack.
package exporttest

//go:generate go run github.com/wspl/demi/go/cmd/wiregen

//demi:enum
//demi:export
type Mode string

const (
	ModeRead  Mode = "read"
	ModeWrite Mode = "write"
)

//demi:union tag=kind
//demi:export
type Choice interface{ choice() }

//demi:variant limited
type Limited struct {
	Count uint16 `json:"count" check:"range=1..3"`
}

func (Limited) choice() {}

//demi:variant empty
type Empty struct{}

func (Empty) choice() {}
