package backend

import (
	"sync"

	"github.com/wspl/demi/go/webapi"
)

// ProviderOperations lets one change of an entry run at a time (providers.md
// § Login and publication): an edit, a deletion, an account change or a
// test holds the entry while it runs, and a device login into the entry
// holds it until the login ends. Another change meanwhile is refused as
// busy. Its zero value holds nothing.
type ProviderOperations struct {
	mu   sync.Mutex
	held map[webapi.ProviderID]struct{}
}

// Reserve holds entry id and returns the function that releases it, or
// false while another change holds it. Releasing twice releases once.
func (o *ProviderOperations) Reserve(id webapi.ProviderID) (release func(), ok bool) {
	o.mu.Lock()
	defer o.mu.Unlock()
	if _, held := o.held[id]; held {
		return nil, false
	}
	if o.held == nil {
		o.held = map[webapi.ProviderID]struct{}{}
	}
	o.held[id] = struct{}{}
	return sync.OnceFunc(func() {
		o.mu.Lock()
		defer o.mu.Unlock()
		delete(o.held, id)
	}), true
}
