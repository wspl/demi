package live

//revive:disable:unused-parameter API checkpoint: retain parameter names for dependent implementers.

import "github.com/wspl/demi/internal/cmdpkg/browser/chrome/tabs"

// Hub is the way to one browser's live view hub. Viewers of a tab share its
// capture and encoding; the hub owns their screen and viewport decisions and
// each watched tab's page observer. It does not own the environment.
// A Hub must be created with Start and must not be copied.
type Hub struct{}

// Start starts the hub of environment; it ends with the environment.
// The browser owner starts exactly one hub per environment. All hub workers
// register with environment.StartTask, and environment.Close cancels and joins
// them. Start returns an error if the environment has stopped admitting work.
func Start(environment *tabs.Environment) (*Hub, error) {
	panic("not written: k-chrome-live")
}
