package backend

import (
	"context"
	"errors"
	"slices"
	"sync"
	"time"

	"github.com/google/uuid"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/provider"
	"github.com/wspl/demi/go/webapi"
)

// LoginTiming says how long a login waits for its user, and how long its
// result is kept.
type LoginTiming struct{ Lifetime, Retention time.Duration }

// DefaultLoginTiming is ten minutes of each.
var DefaultLoginTiming = LoginTiming{Lifetime: 10 * time.Minute, Retention: 10 * time.Minute}

// LoginRefusalKind says why a login did not start; the edge answers each
// with its own status.
type LoginRefusalKind int

const (
	// LoginWithoutFlow: the family has no device login.
	LoginWithoutFlow LoginRefusalKind = iota + 1
	// LoginExists: the owner already holds the family's subscription entry.
	LoginExists
	// LoginBusy: another change holds the entry, or the backend is shutting
	// down.
	LoginBusy
)

// LoginRefusal is a login a rule refused to start. An unknown family is an
// *UnknownFamilyError, and a failure of the store or of the provider's
// assembly is returned as itself.
type LoginRefusal struct {
	Kind    LoginRefusalKind
	Message string
}

func (r *LoginRefusal) Error() string { return r.Message }

func noLoginFlow(family string) *LoginRefusal {
	return &LoginRefusal{LoginWithoutFlow, family + " has no device login"}
}

func loginExists(family string) *LoginRefusal {
	return &LoginRefusal{LoginExists, "This scope already has a " + family + " subscription"}
}

var (
	errLoginBusy      = &LoginRefusal{LoginBusy, "Another provider operation is still running"}
	errLoginExpired   = errors.New("The login expired")
	errLoginCancelled = errors.New("The login was cancelled")
)

// LoginFlows are the device logins of the backend (providers.md § Login and
// publication): each login runs the family's device flow as a goroutine of
// its own, reports what the user must do while it waits, and publishes the
// account when it completes. A login into a new entry authenticates against
// a pool held in memory and publishes the entry with its account in one
// transaction; a login into an existing entry holds the entry until it
// ends. A login expires after its lifetime, cancelling it stops it at once,
// and its result is kept for the retention after it ends.
type LoginFlows struct {
	assembly   *ProviderAssembly
	operations *ProviderOperations
	timing     LoginTiming
	// closing ends at Close, when stop is called; every login's context is
	// its child.
	closing context.Context
	stop    context.CancelFunc
	tasks   *TaskGroup
	// mu guards flows and each flow's state and end: the edge's requests
	// and the login goroutines update them, and no section waits.
	mu    sync.Mutex
	flows map[webapi.LoginID]*loginFlow
}

type loginFlow struct {
	// starter is the user who started the login, who alone reads and
	// cancels it.
	starter webapi.UserID
	state   webapi.LoginState
	// ctx ends when the login is cancelled or the backend closes.
	ctx    context.Context
	cancel context.CancelFunc
	// endedAt is when the login ended; its result goes once the retention
	// has passed.
	endedAt *time.Time
	// ended closes once the login's goroutine has recorded how it ended.
	ended chan struct{}
}

// loginTarget is where a login publishes its account: a new entry of family
// labelled label, published with the staged pool's accounts, or an existing
// entry, held until the login ends.
type loginTarget struct {
	family, label string
	staged        *provider.MemoryCredentialPool
	existing      *ProviderEntry
	// release gives up the existing entry; it does nothing for a new one.
	release func()
}

// NewLoginFlows starts no login yet.
func NewLoginFlows(assembly *ProviderAssembly, operations *ProviderOperations, timing LoginTiming) *LoginFlows {
	closing, stop := context.WithCancel(context.Background())
	return &LoginFlows{assembly: assembly, operations: operations, timing: timing, closing: closing, stop: stop, tasks: newTaskGroup(closing), flows: map[webapi.LoginID]*loginFlow{}}
}

// Start starts starter's device login of family: into existing, or, when
// existing is nil, into a new entry of owner's labelled label. Only starter
// reads and cancels it.
func (l *LoginFlows) Start(ctx context.Context, owner, starter webapi.UserID, family, label string, existing *ProviderEntry) (webapi.LoginID, error) {
	if l.closing.Err() != nil {
		return webapi.LoginID{}, errLoginBusy
	}
	registered, err := l.assembly.Family(family)
	if err != nil {
		return webapi.LoginID{}, err
	}
	if registered.Credential() != webapi.CredentialKindSubscription {
		return webapi.LoginID{}, noLoginFlow(family)
	}
	l.prune()
	id, err := webapi.ParseLoginID(uuid.NewString())
	if err != nil {
		// A UUID is never empty.
		panic(err)
	}
	made, target, err := l.target(ctx, owner, id, family, label, existing)
	if err != nil {
		return webapi.LoginID{}, err
	}
	owned, ok := made.(provider.AccountsProvider)
	if !ok || !owned.Accounts().Capability().Login {
		target.release()
		return webapi.LoginID{}, noLoginFlow(family)
	}
	flowCtx, cancel := context.WithCancel(l.closing)
	flow := &loginFlow{starter: starter, state: webapi.LoginStatePending{}, ctx: flowCtx, cancel: cancel, ended: make(chan struct{})}
	l.mu.Lock()
	l.flows[id] = flow
	l.mu.Unlock()
	started := l.tasks.Go(func(context.Context) { l.run(flow, owner, owned.Accounts(), target) })
	if !started {
		// The backend closed meanwhile.
		l.mu.Lock()
		delete(l.flows, id)
		l.mu.Unlock()
		cancel()
		target.release()
		return webapi.LoginID{}, errLoginBusy
	}
	return id, nil
}

// target is the provider a login runs with and where it publishes: an
// existing entry's own provider, which the login holds, or a provider over
// a staged pool for a new entry, which the owner must not hold yet.
func (l *LoginFlows) target(ctx context.Context, owner webapi.UserID, id webapi.LoginID, family, label string, existing *ProviderEntry) (provider.Provider, loginTarget, error) {
	if existing != nil {
		release, ok := l.operations.Reserve(existing.ID)
		if !ok {
			return nil, loginTarget{}, errLoginBusy
		}
		made, err := l.assembly.ProviderFor(ctx, *existing)
		if err != nil {
			release()
			return nil, loginTarget{}, err
		}
		return made, loginTarget{existing: existing, release: release}, nil
	}
	entries, err := l.assembly.vault.Entries(ctx, owner)
	if err != nil {
		return nil, loginTarget{}, err
	}
	if slices.ContainsFunc(entries, func(entry ProviderEntry) bool { return entry.Family == family }) {
		return nil, loginTarget{}, loginExists(family)
	}
	staged := provider.NewMemoryCredentialPool()
	made, err := l.assembly.Detached(family, id.String(), label, staged)
	if err != nil {
		return nil, loginTarget{}, err
	}
	return made, loginTarget{family: family, label: label, staged: staged, release: func() {}}, nil
}

// State is the login's state, for its starter; false for a login the
// starter does not have.
func (l *LoginFlows) State(id webapi.LoginID, starter webapi.UserID) (webapi.LoginState, bool) {
	l.prune()
	l.mu.Lock()
	defer l.mu.Unlock()
	flow, ok := l.flows[id]
	if !ok || flow.starter != starter {
		return nil, false
	}
	return flow.state, true
}

// Cancel cancels the starter's login and waits until it has ended; false
// for a login the starter does not have. ctx bounds only the wait.
func (l *LoginFlows) Cancel(ctx context.Context, id webapi.LoginID, starter webapi.UserID) (bool, error) {
	l.mu.Lock()
	flow, ok := l.flows[id]
	l.mu.Unlock()
	if !ok || flow.starter != starter {
		return false, nil
	}
	flow.cancel()
	select {
	case <-flow.ended:
		return true, nil
	case <-ctx.Done():
		return true, ctx.Err()
	}
}

// Close cancels every login and waits for them, at shutdown.
func (l *LoginFlows) Close() {
	l.stop()
	l.tasks.close()
	l.tasks.wait()
}

// prune drops the results that have been kept long enough.
func (l *LoginFlows) prune() {
	l.mu.Lock()
	defer l.mu.Unlock()
	for id, flow := range l.flows {
		if flow.endedAt != nil && time.Since(*flow.endedAt) >= l.timing.Retention {
			delete(l.flows, id)
		}
	}
}

// run runs the login until it completes, fails, expires or is cancelled,
// and records how it ended.
func (l *LoginFlows) run(flow *loginFlow, owner webapi.UserID, accounts provider.SubscriptionAccounts, target loginTarget) {
	defer target.release()
	defer flow.cancel()
	wait, stop := context.WithTimeoutCause(flow.ctx, l.timing.Lifetime, errLoginExpired)
	account, err := accounts.Login(wait, func(pending core.LoginPending) { l.pending(flow, pending) })
	expired := errors.Is(context.Cause(wait), errLoginExpired)
	stop()
	var state webapi.LoginState
	switch {
	case flow.ctx.Err() != nil:
		state = webapi.LoginStateFailed{Message: errLoginCancelled.Error()}
	case err != nil && expired:
		state = webapi.LoginStateFailed{Message: errLoginExpired.Error()}
	case err != nil:
		state = webapi.LoginStateFailed{Message: err.Error()}
	default:
		// Publishing runs to its end whatever happens to the login now.
		state = l.publish(context.WithoutCancel(flow.ctx), owner, target, account.ID)
	}
	ended := time.Now()
	l.mu.Lock()
	flow.state = state
	flow.endedAt = &ended
	l.mu.Unlock()
	close(flow.ended)
}

// pending stores what the vendor asked the user to do, while the login
// waits.
func (l *LoginFlows) pending(flow *loginFlow, pending core.LoginPending) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if _, waiting := flow.state.(webapi.LoginStatePending); !waiting || flow.ctx.Err() != nil {
		return
	}
	flow.state = webapi.LoginStatePending{VerificationURL: &pending.VerificationURL, UserCode: pending.UserCode, ExpiresAt: pending.ExpiresAt}
}

// publish publishes the login's account: a new entry with its staged
// accounts, which a concurrent login of the same family may have won, or
// the existing entry, whose provider and catalog then start afresh.
func (l *LoginFlows) publish(ctx context.Context, owner webapi.UserID, target loginTarget, account string) webapi.LoginState {
	credential, err := webapi.ParseCredentialID(account)
	if err != nil {
		return webapi.LoginStateFailed{Message: err.Error()}
	}
	if target.existing != nil {
		if err := l.assembly.Invalidate(ctx, target.existing.ID); err != nil {
			return webapi.LoginStateFailed{Message: err.Error()}
		}
		return webapi.LoginStateCompleted{ProviderID: target.existing.ID, CredentialID: credential}
	}
	entry, err := l.assembly.vault.CreateSubscription(ctx, owner, target.family, target.label, target.staged)
	if err != nil {
		return webapi.LoginStateFailed{Message: err.Error()}
	}
	if entry == nil {
		return webapi.LoginStateFailed{Message: loginExists(target.family).Error()}
	}
	return webapi.LoginStateCompleted{ProviderID: entry.ID, CredentialID: credential}
}
