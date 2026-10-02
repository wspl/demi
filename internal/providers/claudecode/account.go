package claudecode

import (
	"context"
	"crypto/sha256"
	"fmt"

	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
)

//go:generate go run ../../../tools/contractgen

// The `claude-code` family's secret document. Demi defines it; it is not the
// CLI's own credentials file.
// +demi:root
type secretDocument struct {
	// The setup token, which the CLI takes as `CLAUDE_CODE_OAUTH_TOKEN`.
	AccessToken provider.Secret `json:"accessToken"`
}

func (s secretDocument) label() provider.AccountLabel {
	digest := sha256.Sum256([]byte(s.AccessToken.Expose()))
	hex := fmt.Sprintf("%x", digest[:8])
	identity := "token:" + hex
	return provider.AccountLabel{Label: "claude-" + hex[8:], IdentityKey: &identity}
}

type accountKit struct{}

func (accountKit) Capability() provider.AccountsCapability {
	return provider.AccountsCapability{Add: true}
}
func (accountKit) Login(context.Context, func(core.LoginPending)) (provider.NewAccount, error) {
	return provider.NewAccount{}, provider.ErrLoginUnsupported
}
func (accountKit) Add(input provider.AddAccount) (provider.NewAccount, error) {
	secret := secretDocument{AccessToken: input.SetupToken}
	data, err := secret.MarshalJSON()
	if err != nil {
		return provider.NewAccount{}, err
	}
	return provider.NewAccount{Secret: string(data), Label: secret.label()}, nil
}

func (p *Provider) stored(ctx context.Context) (secretDocument, *provider.AuthFailure) {
	if p.config.Account == nil {
		return secretDocument{}, &provider.AuthFailure{Family: family, Reason: provider.AuthReasonMissing}
	}
	stored, err := provider.ReadSecret(ctx, p.pool.Document(*p.config.Account), decodeSecretDocument)
	if err != nil {
		failure := provider.AccountAuthFailure(family, err)
		return secretDocument{}, &failure
	}
	return stored.Secret, nil
}
