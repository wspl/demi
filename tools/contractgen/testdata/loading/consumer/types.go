package consumer

import "github.com/wspl/demi/tools/contractgen/testdata/loading/dependency"

type Handler[T interface{ Validate() error }] struct {
	Value T
}

var InvocationHandler Handler[dependency.Invocation]

// +demi:root
type Request struct {
	Text string `json:"text"`
}
