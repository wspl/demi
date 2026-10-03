package providers

import (
	"context"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/google/uuid"
	"github.com/wspl/demi/internal/backend/database"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/webapi"
)

// LoginTiming is how long a login waits for its user and keeps its result.
type LoginTiming struct {
	// Lifetime limits how long a login waits for its user.
	Lifetime time.Duration
	// Retention limits how long a completed login result remains readable.
	Retention time.Duration
}

// LoginFlows owns the backend login tasks; Close cancels and joins them.
type LoginFlows struct {
	assembly   *Assembly
	operations *Operations
	timing     LoginTiming
	// mu protects flow state and shutdown admission, never provider calls.
	mu      sync.Mutex
	flows   map[webapi.LoginID]*loginFlow
	ctx     context.Context
	cancel  context.CancelFunc
	workers sync.WaitGroup
}
type loginFlow struct {
	owner  webapi.UserID
	state  webapi.LoginState
	cancel context.CancelFunc
	ended  time.Time
	done   chan struct{}
}
type loginTarget struct {
	existing      *ProviderEntry
	staged        *provider.MemoryCredentialPool
	family, label string
	held          *Reservation
}

// Operations admits one configuration operation per provider. The zero value is ready to use.
type Operations struct {
	// mu protects entry reservations; no operation runs while it is held.
	mu   sync.Mutex
	held map[webapi.ProviderID]bool
}

// Reservation holds an entry until Release; do not copy it.
type Reservation struct {
	operations *Operations
	id         webapi.ProviderID
	once       sync.Once
}

// DefaultLoginTiming returns ten minutes for both lifetime and retention.
func DefaultLoginTiming() LoginTiming {
	return LoginTiming{Lifetime: 10 * time.Minute, Retention: 10 * time.Minute}
}

// NewLoginFlows creates the backend login owner.
func NewLoginFlows(assembly *Assembly, operations *Operations, timing LoginTiming) *LoginFlows {
	ctx, cancel := context.WithCancel(context.Background())
	return &LoginFlows{
		assembly:   assembly,
		operations: operations,
		timing:     timing,
		flows:      make(map[webapi.LoginID]*loginFlow),
		ctx:        ctx,
		cancel:     cancel,
	}
}

// Start starts a device login; only starter can read and cancel it.
func (f *LoginFlows) Start(
	ctx context.Context,
	owner, starter webapi.UserID,
	family, label string,
	existing *ProviderEntry,
) (webapi.LoginID, error) {
	f.mu.Lock()
	closed := f.ctx.Err() != nil
	f.mu.Unlock()
	if closed {
		return "", ErrLoginBusy
	}
	if err := f.checkLoginFamily(family); err != nil {
		return "", err
	}
	id, err := newLoginID()
	if err != nil {
		return "", err
	}
	target := loginTarget{existing: existing, family: family, label: label}
	var p provider.Provider
	if existing != nil {
		target.held = f.operations.Reserve(existing.ID)
		if target.held == nil {
			return "", ErrLoginBusy
		}
		defer func() {
			if target.held != nil {
				target.held.Release()
			}
		}()
		p, err = f.assembly.ProviderFor(ctx, *existing)
	} else {
		var refusal error
		p, target.staged, refusal = f.prepareNewLogin(ctx, owner, family, label, id)
		if refusal != nil {
			return "", refusal
		}
	}
	if err != nil {
		return "", &LoginError{Kind: LoginAssembly, Err: err}
	}
	accounts := p.Accounts()
	if accounts == nil || !accounts.Capability().Login {
		return "", &LoginError{Kind: LoginNoLoginFlow, Family: family}
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if f.ctx.Err() != nil {
		return "", ErrLoginBusy
	}
	f.pruneLocked()
	f.registerLoginLocked(id, owner, starter, accounts, target)
	target.held = nil // The task now owns the reservation.
	return id, nil
}

// State returns a login state only to its starter, or nil.
func (f *LoginFlows) State(id webapi.LoginID, owner webapi.UserID) webapi.LoginState {
	f.mu.Lock()
	defer f.mu.Unlock()
	f.pruneLocked()
	flow := f.flows[id]
	if flow == nil || flow.owner != owner {
		return nil
	}
	return copyLoginState(flow.state)
}

// Cancel cancels and joins the starter login, reporting whether it exists.
func (f *LoginFlows) Cancel(_ context.Context, id webapi.LoginID, owner webapi.UserID) bool {
	f.mu.Lock()
	flow := f.flows[id]
	if flow == nil || flow.owner != owner {
		f.mu.Unlock()
		return false
	}
	cancel, done := flow.cancel, flow.done
	f.mu.Unlock()
	cancel()
	// A canceled login must release its reservation before this call returns.
	<-done
	return true
}

// Close cancels and joins all login tasks.
func (f *LoginFlows) Close(_ context.Context) error {
	f.mu.Lock()
	f.cancel()
	f.mu.Unlock()
	f.workers.Wait()
	return nil
}

// Reserve returns the entry's reservation, or nil when another operation holds it.
func (o *Operations) Reserve(id webapi.ProviderID) *Reservation {
	o.mu.Lock()
	defer o.mu.Unlock()
	if o.held[id] {
		return nil
	}
	if o.held == nil {
		o.held = make(map[webapi.ProviderID]bool)
	}
	o.held[id] = true
	return &Reservation{operations: o, id: id}
}

// Release releases the entry reservation. It is idempotent.
func (r *Reservation) Release() {
	r.once.Do(func() {
		r.operations.mu.Lock()
		delete(r.operations.held, r.id)
		r.operations.mu.Unlock()
	})
}

func (f *LoginFlows) pruneLocked() {
	for id, flow := range f.flows {
		if !flow.ended.IsZero() && time.Since(flow.ended) >= f.timing.Retention {
			delete(f.flows, id)
		}
	}
}

func (f *LoginFlows) run(
	ctx context.Context,
	owner webapi.UserID,
	accounts provider.SubscriptionAccounts,
	target loginTarget,
	flow *loginFlow,
) {
	defer flow.cancel()
	account, err := accounts.Login(ctx, func(p core.LoginPending) {
		f.mu.Lock()
		defer f.mu.Unlock()
		if ctx.Err() == nil {
			if _, ok := flow.state.(*webapi.LoginStatePending); ok {
				flow.state = &webapi.LoginStatePending{
					VerificationURL: &p.VerificationURL,
					UserCode:        p.UserCode,
					ExpiresAt:       p.ExpiresAt,
				}
			}
		}
	})
	var state webapi.LoginState
	if ctx.Err() != nil {
		message := "The login was cancelled"
		if errors.Is(ctx.Err(), context.DeadlineExceeded) {
			message = "The login expired"
		}
		state = &webapi.LoginStateFailed{Message: message}
	} else if err != nil {
		state = &webapi.LoginStateFailed{Message: err.Error()}
	} else {
		state = f.publish(context.WithoutCancel(ctx), owner, target, account.ID)
	}
	if target.held != nil {
		target.held.Release()
	}
	f.mu.Lock()
	flow.state = state
	flow.ended = time.Now()
	f.mu.Unlock()
	close(flow.done)
}

func (f *LoginFlows) publish(
	ctx context.Context,
	owner webapi.UserID,
	target loginTarget,
	account string,
) webapi.LoginState {
	credential, err := webapi.ParseCredentialID(account)
	if err != nil {
		return &webapi.LoginStateFailed{Message: err.Error()}
	}
	var id webapi.ProviderID
	if target.existing != nil {
		id = target.existing.ID
		err = f.assembly.Invalidate(ctx, id)
	} else {
		var entry ProviderEntry
		entry, err = f.assembly.vault.CreateSubscription(ctx, owner, target.family, target.label, target.staged)
		if errors.Is(err, database.ErrSubscriptionExists) {
			err = &LoginError{Kind: LoginExists, Family: target.family}
		}
		if err == nil {
			id = entry.ID
		}
	}
	if err != nil {
		return &webapi.LoginStateFailed{Message: err.Error()}
	}
	return &webapi.LoginStateCompleted{ProviderID: id, CredentialID: credential}
}

func copyLoginState(state webapi.LoginState) webapi.LoginState {
	switch s := state.(type) {
	case *webapi.LoginStatePending:
		snapshot := *s
		if s.VerificationURL != nil {
			v := *s.VerificationURL
			snapshot.VerificationURL = &v
		}
		if s.UserCode != nil {
			v := *s.UserCode
			snapshot.UserCode = &v
		}
		if s.ExpiresAt != nil {
			v := *s.ExpiresAt
			snapshot.ExpiresAt = &v
		}
		return &snapshot
	case *webapi.LoginStateCompleted:
		snapshot := *s
		return &snapshot
	case *webapi.LoginStateFailed:
		snapshot := *s
		return &snapshot
	}
	return nil
}

// hasLoginFamily checks whether the user already configured the requested login family.
func hasLoginFamily(entries []ProviderEntry, family string) bool {
	for _, entry := range entries {
		if entry.Family == family {
			return true
		}
	}
	return false
}

// newLoginID creates and validates a login identity.
func newLoginID() (webapi.LoginID, error) {
	randomID, err := uuid.NewRandom()
	if err != nil {
		return "", &LoginError{
			Kind: LoginAssembly,
			Err:  fmt.Errorf("create login identity: %w", err),
		}
	}
	id, err := webapi.ParseLoginID(randomID.String())
	if err != nil {
		return "", &LoginError{Kind: LoginAssembly, Err: err}
	}
	return id, nil
}

// checkLoginFamily refuses unregistered families and families without subscription credentials.
func (f *LoginFlows) checkLoginFamily(family string) error {
	registered, err := f.assembly.Family(family)
	if err != nil {
		return &LoginError{Kind: LoginAssembly, Err: err}
	}
	if registered.Credential() != webapi.CredentialKindSubscription {
		return &LoginError{Kind: LoginNoLoginFlow, Family: family}
	}
	return nil
}

// prepareNewLogin builds a detached subscription provider after checking the user’s existing families.
func (f *LoginFlows) prepareNewLogin(
	ctx context.Context,
	owner webapi.UserID,
	family, label string,
	id webapi.LoginID,
) (provider.Provider, *provider.MemoryCredentialPool, error) {
	entries, err := f.assembly.vault.Entries(ctx, owner)
	if err != nil {
		return nil, nil, &LoginError{Kind: LoginAssembly, Err: err}
	}
	if hasLoginFamily(entries, family) {
		return nil, nil, &LoginError{Kind: LoginExists, Family: family}
	}
	staged := provider.NewMemoryCredentialPool()
	built, err := f.assembly.Detached(family, string(id), label, staged)
	if err != nil {
		return nil, nil, &LoginError{Kind: LoginAssembly, Err: err}
	}
	return built, staged, nil
}

// registerLoginLocked publishes and starts a login while the caller holds the flow mutex.
func (f *LoginFlows) registerLoginLocked(
	id webapi.LoginID,
	owner, starter webapi.UserID,
	accounts provider.SubscriptionAccounts,
	target loginTarget,
) {
	runCtx, cancel := context.WithTimeout(f.ctx, f.timing.Lifetime)
	flow := &loginFlow{
		owner:  starter,
		state:  &webapi.LoginStatePending{},
		cancel: cancel,
		done:   make(chan struct{}),
	}
	f.flows[id] = flow
	owned := target
	f.workers.Go(func() {
		f.run(runCtx, owner, accounts, owned, flow)
	})
}
