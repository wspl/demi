package tools

import (
	"context"
	"errors"
	"fmt"
	"slices"
	"sync"
	"time"

	"github.com/wspl/demi/internal/agent/session"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/types"
)

// Environments owns one node's shell environments and their repeat guards.
// Its zero value is ready for use. Concurrent calls for one Host create one
// environment. The owner must Dispose it and wait for cleanup before release.
type Environments struct {
	mu       sync.Mutex
	slots    []*environmentSlot
	disposed bool
}

type environmentSlot struct {
	key         host.Key
	environment host.ShellEnvironment
	creating    chan struct{}
	retired     bool
	closeMu     sync.Mutex
	closeDone   chan struct{}
	closeErr    error
	repeatMu    sync.Mutex
	script      string
	count       uint32
	at          time.Time
}

// errDisposedShells refuses work after the node's shells are disposed.
//
//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
var errDisposedShells = errors.New("The node's shells are closed")

// resolve shares creation for one Host and checks a handle against every owner.
func (e *Environments) resolve(
	ctx context.Context,
	key host.Key,
	shell *types.ShellID,
	command *types.CommandID,
	create func(context.Context) (host.ShellEnvironment, error),
) (*environmentSlot, host.ShellEnvironment, error) {
	for {
		e.mu.Lock()
		if e.disposed {
			e.mu.Unlock()
			return nil, nil, errDisposedShells
		}
		slot := e.slotLocked(key)
		if ready := slot.creating; ready != nil {
			e.mu.Unlock()
			select {
			case <-ctx.Done():
				return nil, nil, ctx.Err()
			case <-ready:
				continue
			}
		}
		environment := slot.environment
		if environment == nil {
			var err error
			environment, err = e.createEnvironmentLocked(ctx, slot, create)
			if err != nil {
				return nil, nil, err
			}
		} else {
			e.mu.Unlock()
		}
		if err := e.checkHandleOwner(key, shell, command); err != nil {
			return nil, nil, err
		}
		e.mu.Lock()
		retired := slot.retired || e.disposed
		e.mu.Unlock()
		if retired {
			return nil, nil, errDisposedShells
		}
		return slot, environment, nil
	}
}

type environmentSnapshot struct {
	slot        *environmentSlot
	environment host.ShellEnvironment
}

// snapshot takes the environments without holding a lock while calling them.
func (e *Environments) snapshot() []environmentSnapshot {
	e.mu.Lock()
	defer e.mu.Unlock()
	var result []environmentSnapshot
	for _, slot := range e.slots {
		if slot.environment != nil && slot.creating == nil && !slot.retired {
			result = append(result, environmentSnapshot{slot, slot.environment})
		}
	}
	return result
}

// PageViews returns the pages' view of every command held by the environments.
func (e *Environments) PageViews() []host.PageView {
	var views []host.PageView
	for _, entry := range e.snapshot() {
		views = append(views, entry.environment.PageViews()...)
	}
	return views
}

// Owning returns the environment that owns command, or nil when none does.
func (e *Environments) Owning(command types.CommandID) host.ShellEnvironment {
	for _, entry := range e.snapshot() {
		if entry.environment.OwnsCommand(command) {
			return entry.environment
		}
	}
	return nil
}

// EndAll ends every shell and forgets its environment, joining owned work.
// The next call on a Host makes a fresh environment. Use a cleanup context
// that remains usable after action cancellation.
func (e *Environments) EndAll(ctx context.Context) error { return e.end(ctx, false) }

// Dispose permanently closes the node's shells, including environments being
// created concurrently, and joins their work before returning. Use a cleanup
// context that remains usable after action cancellation.
func (e *Environments) Dispose(ctx context.Context) error { return e.end(ctx, true) }

// end retires the selected environments atomically before joining creation and cleanup.
func (e *Environments) end(ctx context.Context, permanent bool) error {
	e.mu.Lock()
	e.disposed = e.disposed || permanent
	slots := slices.Clone(e.slots)
	waiting := make([]<-chan struct{}, len(slots))
	for i, slot := range slots {
		slot.retired = true
		waiting[i] = slot.creating
	}
	e.mu.Unlock()
	// Cleanup owns and joins every closer, including when one fails.
	var wg sync.WaitGroup
	errs := make([]error, len(slots))
	for i, slot := range slots {
		wg.Go(func() {
			if waiting[i] != nil {
				<-waiting[i]
			}
			errs[i] = slot.close(context.WithoutCancel(ctx))
		})
	}
	wg.Wait()
	// Keep retired slots discoverable by overlapping cleanup until they settle.
	if !permanent {
		e.mu.Lock()
		e.slots = slices.DeleteFunc(e.slots, func(slot *environmentSlot) bool { return slices.Contains(slots, slot) })
		e.mu.Unlock()
	}
	return errors.Join(errs...)
}

// close releases an environment exactly once, after its creation has settled.
func (s *environmentSlot) close(ctx context.Context) error {
	s.closeMu.Lock()
	if done := s.closeDone; done != nil {
		s.closeMu.Unlock()
		<-done
		return s.closeErr
	}
	done := make(chan struct{})
	s.closeDone = done
	s.closeMu.Unlock()
	if s.environment != nil {
		s.closeErr = s.environment.DisposeAll(ctx)
	}
	close(done)
	return s.closeErr
}

// repeated counts consecutive scripts on this Host within sixty seconds and
// returns the suppression outcome and true from the seventh repeat on.
func (s *environmentSlot) repeated(script string) (session.ToolOutcome, bool) {
	s.repeatMu.Lock()
	defer s.repeatMu.Unlock()
	now := time.Now()
	if s.count != 0 && s.script == script && now.Sub(s.at) <= 60*time.Second {
		s.count++
	} else {
		s.count = 1
	}
	s.script = script
	s.at = now
	if s.count <= 6 {
		return session.ToolOutcome{}, false
	}
	text := fmt.Sprintf(
		"Repeated identical shell_exec suppressed.\n"+
			"The same script has been run %d consecutive times in this agent session.\n"+
			"Inspect the previous output, use a different command, "+
			"or provide the final answer instead of repeating it.",
		s.count,
	)
	return session.ToolOutcome{
		Output:  []provider.ResultPart{&provider.TextPart{Text: text}},
		IsError: true,
		View:    &types.RepeatedShellExec{Script: script, Count: s.count},
	}, true
}

// slotLocked finds or adds the current Host slot while the caller holds e.mu.
func (e *Environments) slotLocked(key host.Key) *environmentSlot {
	var slot *environmentSlot
	for _, candidate := range e.slots {
		if candidate.key == key && !candidate.retired {
			slot = candidate
			break
		}
	}
	if slot == nil {
		slot = &environmentSlot{key: key}
		e.slots = append(e.slots, slot)
	}
	return slot
}

// createEnvironmentLocked enters with e.mu held and returns with it released.
// Creation stays owned until a late environment is closed after retirement.
func (e *Environments) createEnvironmentLocked(
	ctx context.Context, slot *environmentSlot, create func(context.Context) (host.ShellEnvironment, error),
) (host.ShellEnvironment, error) {
	ready := make(chan struct{})
	slot.creating = ready
	e.mu.Unlock()
	made, err := create(ctx)
	e.mu.Lock()
	retired := slot.retired || e.disposed
	slot.environment = made
	e.mu.Unlock()
	if retired && made != nil {
		// Creation remains owned until its late environment is closed.
		err = errors.Join(err, slot.close(context.WithoutCancel(ctx)))
	}
	e.mu.Lock()
	slot.creating = nil
	close(ready)
	e.mu.Unlock()
	if retired {
		return nil, errors.Join(errDisposedShells, err)
	}
	if err != nil {
		return nil, err
	}
	return made, nil
}

// checkHandleOwner rejects duplicate handles and handles held by another Host.
func (e *Environments) checkHandleOwner(key host.Key, shell *types.ShellID, command *types.CommandID) error {
	owners := e.snapshot()
	var owner *environmentSlot
	for _, candidate := range owners {
		owns := shell != nil && candidate.environment.OwnsShell(*shell) ||
			command != nil && candidate.environment.OwnsCommand(*command)
		if !owns {
			continue
		}
		if owner != nil {
			if shell != nil {
				//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
				return fmt.Errorf("Shell id \"%s\" is not unique in this session", *shell)
			}
			//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
			return fmt.Errorf("Command id \"%s\" is not unique in this session", *command)
		}
		owner = candidate.slot
	}
	if owner != nil && owner.key != key {
		id := ""
		if shell != nil {
			id = string(*shell)
		} else if command != nil {
			id = string(*command)
		}
		//nolint:staticcheck // ST1005: the text is a product message shown to the user as written.
		return fmt.Errorf("Shell handle \"%s\" belongs to a different Host", id)
	}
	return nil
}
