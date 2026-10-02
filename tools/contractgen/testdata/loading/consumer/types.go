package consumer

import (
	protocol "github.com/chromedp/cdproto/cdp"
	"github.com/wspl/demi/tools/contractgen/testdata/loading/dependency"
)

// An aliased import used only in a body becomes unused when generation strips it.
func frameID() string {
	return string(protocol.FrameID("frame"))
}

type Handler[T interface{ Validate() error }] struct {
	Value T
}

var InvocationHandler Handler[dependency.Invocation]

// +demi:root
type Request struct {
	Text string `json:"text"`
}
