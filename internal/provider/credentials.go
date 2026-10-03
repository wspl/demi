package provider

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"
	"sort"
	"sync"

	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/gates"
)

// AccountMeta is an account's public metadata, never token material.
type AccountMeta struct {
	ID          string
	Label       string
	Detail      *string
	UpdatedAt   core.Timestamp
	Source      string
	IdentityKey *string
}

// Info returns the metadata the web app sees.
func (m AccountMeta) Info() core.AccountInfo {
	return core.AccountInfo{ID: m.ID, Label: m.Label, Detail: m.Detail, UpdatedAt: &m.UpdatedAt}
}

// ErrNoAccount means the pool has no account with the given ID.
var ErrNoAccount = errors.New("no account")

// PoolError reports a failed credential store operation, without secret text.
type PoolError struct{ Err error }

// Error returns the diagnostic for this failure.
func (e *PoolError) Error() string { return e.Err.Error() }

// Unwrap returns the underlying cause.
func (e *PoolError) Unwrap() error { return e.Err }

// Revision is a secret document and its equality-only version.
type Revision struct {
	Text    string
	Version uint64
}

// AccountDocument provides versioned secret storage and serialized refresh turns.
type AccountDocument interface {
	Name() string
	Read(context.Context) (Revision, bool, error)
	Replace(context.Context, string, uint64) (bool, error)
	RefreshTurn(context.Context) (*gates.Permit, error)
}

// CredentialPool is the accounts of one provider entry and its active selection.
type CredentialPool interface {
	List(context.Context) ([]AccountMeta, error)
	Meta(context.Context, string) (AccountMeta, bool, error)
	Active(context.Context) (string, bool, error)
	SetActive(context.Context, string) error
	Write(context.Context, AccountMeta, string) error
	Document(string) AccountDocument
	Remove(context.Context, string) error
}

// FindByIdentity finds an account by its family's identity key; ok is false when there is none.
func FindByIdentity(ctx context.Context, pool CredentialPool, identityKey string) (AccountMeta, bool, error) {
	accounts, err := pool.List(ctx)
	if err != nil {
		return AccountMeta{}, false, err
	}
	for _, account := range accounts {
		if account.IdentityKey != nil && *account.IdentityKey == identityKey {
			return account, true, nil
		}
	}
	return AccountMeta{}, false, nil
}

// CredentialIDFor derives a stable account ID from identity, falling back to label.
func CredentialIDFor(identityKey *string, label string) string {
	if identityKey != nil && *identityKey != "" {
		label = *identityKey
	}
	digest := sha256.Sum256([]byte(label))
	return fmt.Sprintf("cred-%x", digest[:8])
}

// RefreshGates admits one refresher per account in FIFO order. Zero is ready to use.
type RefreshGates struct{ serial gates.KeyedSerial[string] }

// Turn waits for an account's refresh permit. The caller defers Release.
func (g *RefreshGates) Turn(ctx context.Context, account string) (*gates.Permit, error) {
	return g.serial.Acquire(ctx, account)
}

// MemoryCredentialPool holds accounts and refresh gates in memory. Do not copy it.
type MemoryCredentialPool struct {
	mu       sync.Mutex
	accounts map[string]memoryEntry
	active   *string
	gates    RefreshGates
}
type memoryEntry struct {
	meta    AccountMeta
	text    string
	version uint64
}

// NewMemoryCredentialPool returns an empty pool.
func NewMemoryCredentialPool() *MemoryCredentialPool { return &MemoryCredentialPool{} }

// AccountEntry is an account together with its secret document for login publication.
type AccountEntry struct {
	Meta   AccountMeta
	Secret string
}

// Entries returns every account with its secret, ordered by ID.
func (p *MemoryCredentialPool) Entries() []AccountEntry {
	p.mu.Lock()
	defer p.mu.Unlock()
	entries := make([]AccountEntry, 0, len(p.accounts))
	for _, entry := range p.accounts {
		entries = append(entries, AccountEntry{Meta: cloneMeta(entry.meta), Secret: entry.text})
	}
	sort.Slice(entries, func(i, j int) bool { return entries[i].Meta.ID < entries[j].Meta.ID })
	return entries
}

// List returns public metadata in ID order.
func (p *MemoryCredentialPool) List(ctx context.Context) ([]AccountMeta, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	entries := p.Entries()
	result := make([]AccountMeta, len(entries))
	for i, entry := range entries {
		result[i] = entry.Meta
	}
	return result, nil
}

// Meta reads an account's metadata; ok is false when there is none.
func (p *MemoryCredentialPool) Meta(ctx context.Context, id string) (AccountMeta, bool, error) {
	if err := ctx.Err(); err != nil {
		return AccountMeta{}, false, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	entry, ok := p.accounts[id]
	if !ok {
		return AccountMeta{}, false, nil
	}
	return cloneMeta(entry.meta), true, nil
}

// Active reads the selected account ID; ok is false when there is none.
func (p *MemoryCredentialPool) Active(ctx context.Context) (string, bool, error) {
	if err := ctx.Err(); err != nil {
		return "", false, err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.active == nil {
		return "", false, nil
	}
	return *p.active, true, nil
}

// SetActive selects an account the pool contains.
func (p *MemoryCredentialPool) SetActive(ctx context.Context, id string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if _, ok := p.accounts[id]; !ok {
		return fmt.Errorf("%w %s", ErrNoAccount, id)
	}
	p.active = &id
	return nil
}

// Write inserts or replaces an account and increments its revision.
func (p *MemoryCredentialPool) Write(ctx context.Context, meta AccountMeta, secret string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	if p.accounts == nil {
		p.accounts = make(map[string]memoryEntry)
	}
	version := p.accounts[meta.ID].version + 1
	p.accounts[meta.ID] = memoryEntry{meta: cloneMeta(meta), text: secret, version: version}
	return nil
}

// Document returns a handle even when the account does not yet exist.
func (p *MemoryCredentialPool) Document(id string) AccountDocument {
	return &memoryDocument{pool: p, id: id}
}

// Remove deletes an account and any active selection of it.
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

// cloneMeta keeps memory-pool metadata isolated from callers' pointer mutations.
func cloneMeta(m AccountMeta) AccountMeta {
	if m.Detail != nil {
		value := *m.Detail
		m.Detail = &value
	}
	if m.IdentityKey != nil {
		value := *m.IdentityKey
		m.IdentityKey = &value
	}
	return m
}

type memoryDocument struct {
	pool *MemoryCredentialPool
	id   string
}

// Name returns the credential identity.
func (d *memoryDocument) Name() string {
	return "account " + d.id
}

// Read returns the stored credential snapshot; ok is false when there is none.
func (d *memoryDocument) Read(ctx context.Context) (Revision, bool, error) {
	if err := ctx.Err(); err != nil {
		return Revision{}, false, err
	}
	d.pool.mu.Lock()
	defer d.pool.mu.Unlock()
	entry, ok := d.pool.accounts[d.id]
	if !ok {
		return Revision{}, false, nil
	}
	return Revision{Text: entry.text, Version: entry.version}, true, nil
}

// Replace writes credentials when the expected revision still matches.
func (d *memoryDocument) Replace(ctx context.Context, text string, version uint64) (bool, error) {
	if err := ctx.Err(); err != nil {
		return false, err
	}
	d.pool.mu.Lock()
	defer d.pool.mu.Unlock()
	entry, ok := d.pool.accounts[d.id]
	if !ok || entry.version != version {
		return false, nil
	}
	entry.text = text
	entry.version++
	d.pool.accounts[d.id] = entry
	return true, nil
}

// RefreshTurn acquires exclusive ownership of a credential refresh.
func (d *memoryDocument) RefreshTurn(ctx context.Context) (*gates.Permit, error) {
	return d.pool.gates.Turn(ctx, d.id)
}

// ErrNoSecretDocument means the account has no secret document.
var ErrNoSecretDocument = errors.New("the account has no secret document")

// RenewError distinguishes a refused refresh from a failed account operation.
type RenewError struct{ Err error }

// Error returns the diagnostic for this failure.
func (e *RenewError) Error() string { return e.Err.Error() }

// Unwrap returns the underlying cause.
func (e *RenewError) Unwrap() error { return e.Err }

// Stored is a decoded secret and the version it was read at.
type Stored[S any] struct {
	Secret  S
	Version uint64
}

// ReadSecret reads a secret using the family's generated decoder.
func ReadSecret[S any](ctx context.Context, doc AccountDocument, decode func([]byte) (S, error)) (Stored[S], error) {
	var zero Stored[S]
	revision, ok, err := doc.Read(ctx)
	if err != nil {
		return zero, err
	}
	if !ok {
		return zero, ErrNoSecretDocument
	}
	secret, err := DecodeSecretDocument(revision.Text, decode)
	if err != nil {
		return zero, fmt.Errorf("the account's secret document is %w", err)
	}
	return Stored[S]{Secret: secret, Version: revision.Version}, nil
}

// Renew follows the single refresh protocol: reread under the account's turn,
// refresh only if still due, CAS the result, and adopt a competing writer's tokens.
func Renew[S any](
	ctx context.Context,
	doc AccountDocument,
	decode func([]byte) (S, error),
	due func(S) bool,
	refresh func(context.Context, S) (S, error),
) (S, error) {
	var zero S
	stored, err := ReadSecret(ctx, doc, decode)
	if err != nil {
		return zero, err
	}
	if !due(stored.Secret) {
		return stored.Secret, nil
	}
	permit, err := doc.RefreshTurn(ctx)
	if err != nil {
		return zero, err
	}
	defer permit.Release()
	latest, err := ReadSecret(ctx, doc, decode)
	if err != nil {
		return zero, err
	}
	if !due(latest.Secret) {
		return latest.Secret, nil
	}
	next, refreshErr := refresh(ctx, latest.Secret)
	if err := ctx.Err(); err != nil {
		return zero, err
	}
	if refreshErr != nil {
		stored, err = ReadSecret(ctx, doc, decode)
		if err != nil {
			return zero, err
		}
		if stored.Version == latest.Version {
			return zero, &RenewError{Err: refreshErr}
		}
		return stored.Secret, nil
	}
	encoded, err := contract.EncodeJSON(next)
	if err != nil {
		return zero, fmt.Errorf(
			"the account's secret document is %w",
			&SecretDecodeError{Path: ".", Fault: SecretShape},
		)
	}
	kept, err := doc.Replace(ctx, string(encoded), latest.Version)
	if err != nil {
		return zero, err
	}
	if kept {
		return next, nil
	}
	stored, err = ReadSecret(ctx, doc, decode)
	return stored.Secret, err
}

// ErrActiveAccount refuses removing the account currently used for inference.
var ErrActiveAccount = errors.New("the active account cannot be removed; select another account first")

// ErrAccountsUnsupported means the family cannot add accounts this way.
var ErrAccountsUnsupported = errors.New("this provider does not add accounts this way")

// ErrLoginUnsupported means the family has no device login.
var ErrLoginUnsupported = errors.New("this provider has no device login")
