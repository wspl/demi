package live

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import (
	"context"

	"github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

// ViewedBrowser is the conversation's browser as its viewers reach it, which
// the browser program implements. The live view does not know how
// conversations are owned or released. Methods must be safe for concurrent use.
type ViewedBrowser interface {
	// Running returns the running browser and its live view hub, or two nil
	// pointers while no browser runs. It never starts a browser. Once the
	// conversation is released it returns a *cdp.BrowserError with Kind
	// cdp.KindClosed. Waiting honors ctx.
	Running(ctx context.Context) (*tabs.Environment, *Hub, error)

	// Changed returns a notification that closes once a browser starts or
	// ends after this call, including when the browser owner ends. Call it
	// before Running to avoid missing a concurrent transition. Obtain a new
	// notification after each wake; the caller must not close the channel.
	Changed() <-chan struct{}

	// Released closes when the conversation's release arrives. It remains
	// closed thereafter; the caller must not close the channel.
	Released() <-chan struct{}
}

// Serve serves one view of browser, the browser.live invocation carried by
// invocation, until the page or conversation ends it or ctx is canceled.
// It releases this viewer's held input and joins invocation workers before
// returning. Protocol refusal returns completion exit code 2 with invalid_input;
// ordinary completion returns exit code 0, and cancellation returns an error
// matching context.Canceled. The SDK owns the invocation input and output.
func Serve(ctx context.Context, browser ViewedBrowser, invocation cmdsdk.InvocationContext[commandwire.Invocation]) (commandwire.Completion, error) {
	panic("not written: k-chrome-live")
}
