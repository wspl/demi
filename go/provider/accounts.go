package provider

import (
	"context"
	"errors"
	"fmt"
	"sort"
	"strings"
	"sync"

	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/gates"
)

type AccountMeta struct {
	ID, Label   string
	Detail      *string
	UpdatedAt   core.Timestamp
	Source      string
	IdentityKey *string
}

func (m AccountMeta) Info() core.AccountInfo {
	return core.AccountInfo{ID: m.ID, Label: m.Label, Detail: cloneString(m.Detail), UpdatedAt: &m.UpdatedAt}
}

type PoolNotFound struct{ ID string }

func (e PoolNotFound) Error() string { return fmt.Sprintf("no account %s", e.ID) }

type CredentialPool interface {
	List(context.Context) ([]AccountMeta, error)
	Meta(context.Context, string) (*AccountMeta, error)
	Active(context.Context) (*string, error)
	SetActive(context.Context, string) error
	Write(context.Context, AccountMeta, string) error
	Document(string) AccountDocument
	Remove(context.Context, string) error
}

func FindByIdentity(ctx context.Context, pool CredentialPool, key string) (*AccountMeta, error) {
	accounts, err := pool.List(ctx)
	if err != nil {
		return nil, err
	}
	for _, account := range accounts {
		if account.IdentityKey != nil && *account.IdentityKey == key {
			return &account, nil
		}
	}
	return nil, nil
}

// cloneString isolates optional account metadata across the pool boundary.
func cloneString(s *string) *string {
	if s == nil {
		return nil
	}
	value := strings.Clone(*s)
	return &value
}
func cloneAccount(m AccountMeta) AccountMeta {
	m.Detail = cloneString(m.Detail)
	m.IdentityKey = cloneString(m.IdentityKey)
	return m
}

type memoryAccount struct {
	meta     AccountMeta
	revision Revision
}

// MemoryCredentialPool holds login drafts and test accounts. Use its constructor.
type MemoryCredentialPool struct {
	mu       sync.Mutex
	accounts map[string]memoryAccount
	active   *string
	gates    *RefreshGates
}

func NewMemoryCredentialPool() *MemoryCredentialPool {
	return &MemoryCredentialPool{accounts: make(map[string]memoryAccount), gates: NewRefreshGates()}
}
func (p *MemoryCredentialPool) List(ctx context.Context) ([]AccountMeta, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	out := make([]AccountMeta, 0, len(p.accounts))
	for _, entry := range p.accounts {
		out = append(out, cloneAccount(entry.meta))
	}
	sort.Slice(out, func(i, j int) bool { return out[i].ID < out[j].ID })
	return out, nil
}
func (p *MemoryCredentialPool) Meta(ctx context.Context, id string) (*AccountMeta, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	entry, ok := p.accounts[id]
	if !ok {
		return nil, nil
	}
	meta := cloneAccount(entry.meta)
	return &meta, nil
}
func (p *MemoryCredentialPool) Active(ctx context.Context) (*string, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	return cloneString(p.active), nil
}
func (p *MemoryCredentialPool) SetActive(ctx context.Context, id string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if _, ok := p.accounts[id]; !ok {
		return PoolNotFound{ID: id}
	}
	p.active = &id
	return nil
}
func (p *MemoryCredentialPool) Write(ctx context.Context, meta AccountMeta, secret string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	version := p.accounts[meta.ID].revision.Version + 1
	p.accounts[meta.ID] = memoryAccount{meta: cloneAccount(meta), revision: Revision{Text: secret, Version: version}}
	return nil
}
func (p *MemoryCredentialPool) Remove(ctx context.Context, id string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	delete(p.accounts, id)
	if p.active != nil && *p.active == id {
		p.active = nil
	}
	return nil
}

type AccountEntry struct {
	Meta   AccountMeta
	Secret string
}

func (p *MemoryCredentialPool) Entries() []AccountEntry {
	p.mu.Lock()
	defer p.mu.Unlock()
	out := make([]AccountEntry, 0, len(p.accounts))
	for _, entry := range p.accounts {
		out = append(out, AccountEntry{Meta: cloneAccount(entry.meta), Secret: entry.revision.Text})
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Meta.ID < out[j].Meta.ID })
	return out
}
func (p *MemoryCredentialPool) Document(id string) AccountDocument {
	return &memoryDocument{pool: p, id: id}
}

type memoryDocument struct {
	pool *MemoryCredentialPool
	id   string
}

func (d *memoryDocument) Name() string { return "account " + d.id }
func (d *memoryDocument) Read(ctx context.Context) (*Revision, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	d.pool.mu.Lock()
	defer d.pool.mu.Unlock()
	entry, ok := d.pool.accounts[d.id]
	if !ok {
		return nil, nil
	}
	return &entry.revision, nil
}
func (d *memoryDocument) Replace(ctx context.Context, text string, version uint64) (bool, error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	d.pool.mu.Lock()
	defer d.pool.mu.Unlock()
	entry, ok := d.pool.accounts[d.id]
	if !ok || entry.revision.Version != version {
		return false, nil
	}
	entry.revision = Revision{Text: text, Version: version + 1}
	d.pool.accounts[d.id] = entry
	return true, nil
}
func (d *memoryDocument) RefreshTurn(ctx context.Context) (*gates.KeyedPermit[string], error) {
	return d.pool.gates.Acquire(ctx, d.id)
}

type AccountsCapability struct{ Login, Add bool }
type AddAccount struct{ SetupToken Secret }
type AccountLabel struct {
	Label               string
	Detail, IdentityKey *string
}
type NewAccount struct {
	Secret string
	Label  AccountLabel
}

var (
	ErrActiveAccount    = errors.New("the active account cannot be removed; select another account first")
	ErrAddUnsupported   = errors.New("this provider does not add accounts this way")
	ErrLoginUnsupported = errors.New("this provider has no device login")
)

type AccountKit interface {
	Capability() AccountsCapability
	Login(context.Context, func(core.LoginPending)) (NewAccount, error)
	Add(AddAccount) (NewAccount, error)
}
type SubscriptionAccounts interface {
	Capability() AccountsCapability
	List(context.Context) ([]core.AccountInfo, error)
	Active(context.Context) (*string, error)
	SetActive(context.Context, string) error
	Login(context.Context, func(core.LoginPending)) (core.AccountInfo, error)
	Add(context.Context, AddAccount) (core.AccountInfo, error)
	Remove(context.Context, string) error
}
type Accounts struct {
	pool  CredentialPool
	kit   AccountKit
	clock core.Clock
}

func NewAccounts(pool CredentialPool, kit AccountKit, clock core.Clock) *Accounts {
	return &Accounts{pool: pool, kit: kit, clock: clock}
}
func (a *Accounts) Capability() AccountsCapability { return a.kit.Capability() }
func (a *Accounts) List(ctx context.Context) ([]core.AccountInfo, error) {
	metas, err := a.pool.List(ctx)
	if err != nil {
		return nil, err
	}
	out := make([]core.AccountInfo, 0, len(metas))
	for _, meta := range metas {
		out = append(out, meta.Info())
	}
	return out, nil
}
func (a *Accounts) Active(ctx context.Context) (*string, error)    { return a.pool.Active(ctx) }
func (a *Accounts) SetActive(ctx context.Context, id string) error { return a.pool.SetActive(ctx, id) }
func (a *Accounts) importAccount(ctx context.Context, account NewAccount, source string) (core.AccountInfo, error) {
	label := account.Label
	key := ""
	if label.IdentityKey != nil {
		key = *label.IdentityKey
	}
	id := CredentialIDFor(key, label.Label)
	if label.IdentityKey != nil {
		existing, err := FindByIdentity(ctx, a.pool, key)
		if err != nil {
			return core.AccountInfo{}, err
		}
		if existing != nil {
			id = existing.ID
		}
	}
	meta := AccountMeta{ID: id, Label: label.Label, Detail: label.Detail, UpdatedAt: a.clock.Now(), Source: source, IdentityKey: label.IdentityKey}
	if err := a.pool.Write(ctx, meta, account.Secret); err != nil {
		return core.AccountInfo{}, err
	}
	active, err := a.pool.Active(ctx)
	if err != nil {
		return core.AccountInfo{}, err
	}
	if active == nil {
		if err := a.pool.SetActive(ctx, id); err != nil {
			return core.AccountInfo{}, err
		}
	}
	return meta.Info(), nil
}
func (a *Accounts) Login(ctx context.Context, pending func(core.LoginPending)) (core.AccountInfo, error) {
	if !a.Capability().Login {
		return core.AccountInfo{}, ErrLoginUnsupported
	}
	account, err := a.kit.Login(ctx, pending)
	if err != nil {
		return core.AccountInfo{}, err
	}
	if err := ctx.Err(); err != nil {
		return core.AccountInfo{}, err
	}
	return a.importAccount(ctx, account, "login:device")
}
func (a *Accounts) Add(ctx context.Context, input AddAccount) (core.AccountInfo, error) {
	if !a.Capability().Add {
		return core.AccountInfo{}, ErrAddUnsupported
	}
	account, err := a.kit.Add(input)
	if err != nil {
		return core.AccountInfo{}, err
	}
	return a.importAccount(ctx, account, "add")
}
func (a *Accounts) Remove(ctx context.Context, id string) error {
	active, err := a.pool.Active(ctx)
	if err != nil {
		return err
	}
	if active != nil && *active == id {
		return ErrActiveAccount
	}
	meta, err := a.pool.Meta(ctx, id)
	if err != nil {
		return err
	}
	if meta == nil {
		return PoolNotFound{ID: id}
	}
	return a.pool.Remove(ctx, id)
}
