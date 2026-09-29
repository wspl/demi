package provider

import (
	"context"
	"crypto/sha256"
	"errors"
	"fmt"

	"github.com/wspl/demi/go/gates"
)

// Revision is one version of a family's opaque secret document.
type Revision struct {
	Text    string
	Version uint64
}

// AccountDocument is the pool's versioned document boundary. A nil revision
// means the account has no document. Every acquired permit must be released.
type AccountDocument interface {
	Name() string
	Read(context.Context) (*Revision, error)
	Replace(context.Context, string, uint64) (bool, error)
	RefreshTurn(context.Context) (*gates.KeyedPermit[string], error)
}

// RefreshGates serializes refreshes per account, using the shared FIFO gate.
// The owner includes the provider entry in the account key.
type RefreshGates = gates.KeyedSerialGate[string]

func NewRefreshGates() *RefreshGates { return gates.NewKeyedSerialGate[string]() }

var ErrMissingDocument = errors.New("the account has no secret document")

// Stored holds the decoded secret and the revision used for a conditional write.
type Stored[S any] struct {
	Secret  S
	Version uint64
}

// ReadSecret reads a document through its family's strict, secret-safe decoder.
func ReadSecret[S any](ctx context.Context, doc AccountDocument, decode func([]byte) (S, error)) (Stored[S], error) {
	var zero Stored[S]
	revision, err := doc.Read(ctx)
	if err != nil {
		return zero, err
	}
	if revision == nil {
		return zero, ErrMissingDocument
	}
	secret, err := decode([]byte(revision.Text))
	if err != nil {
		return zero, fmt.Errorf("the account's secret document is %w", err)
	}
	return Stored[S]{Secret: secret, Version: revision.Version}, nil
}

// Renew refreshes a due secret once, rereading under the account's turn and
// yielding to a competing writer after either a refusal or a lost replace.
func Renew[S any](ctx context.Context, doc AccountDocument, decode func([]byte) (S, error), encode func(S) ([]byte, error), due func(S) bool, refresh func(context.Context, S) (S, error)) (S, error) {
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
		winner, err := ReadSecret(ctx, doc, decode)
		if err != nil {
			return zero, err
		}
		if winner.Version == latest.Version {
			return zero, refreshErr
		}
		return winner.Secret, nil
	}
	text, err := encode(next)
	if err != nil {
		return zero, err
	}
	kept, err := doc.Replace(ctx, string(text), latest.Version)
	if err != nil {
		return zero, err
	}
	if kept {
		return next, nil
	}
	winner, err := ReadSecret(ctx, doc, decode)
	return winner.Secret, err
}

// CredentialIDFor gives the same account the same ID in every importing pool.
func CredentialIDFor(identityKey, label string) string {
	if identityKey == "" {
		identityKey = label
	}
	digest := sha256.Sum256([]byte(identityKey))
	return fmt.Sprintf("cred-%x", digest[:8])
}
