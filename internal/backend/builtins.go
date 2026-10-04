package backend

import (
	"context"

	"github.com/wspl/demi/internal/backend/providerhost"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugin/browser"
	"github.com/wspl/demi/internal/plugin/changes"
	"github.com/wspl/demi/internal/plugin/expose"
	"github.com/wspl/demi/internal/plugin/file"
	"github.com/wspl/demi/internal/plugin/filebrowser"
	"github.com/wspl/demi/internal/plugin/skills"
	"github.com/wspl/demi/internal/plugin/todo"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/anthropicapi"
	"github.com/wspl/demi/internal/provider/claudecode"
	"github.com/wspl/demi/internal/provider/codex"
	"github.com/wspl/demi/internal/provider/google"
	"github.com/wspl/demi/internal/provider/grokbuild"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/types"
	"github.com/wspl/demi/internal/webapiproto"
)

// BuiltinFamilies returns the families built into the backend. Tests can
// register additional scripted families in the returned registry.
func BuiltinFamilies() *providerhost.FamilyRegistry {
	registry := &providerhost.FamilyRegistry{}
	registry.Register("anthropic", apiFamily{name: "anthropic"})
	registry.Register(providerhost.SetupTokenFamily, claudeFamily{})
	registry.Register("codex", subscriptionFamily{name: "codex"})
	registry.Register("google", apiFamily{name: "google"})
	registry.Register("grok-build", subscriptionFamily{name: "grok-build"})
	registry.Register("openai", apiFamily{name: "openai"})
	return registry
}

type apiFamily struct{ name string }

// Credential identifies the credentials this provider family accepts.
func (apiFamily) Credential() webapiproto.CredentialKind { return webapiproto.CredentialKindAPIKey }

// Wires lists the selectable wire protocols for this family.
func (f apiFamily) Wires() []types.WireAPI {
	if f.name == "openai" {
		return []types.WireAPI{types.WireAPIResponses, types.WireAPIChatCompletions}
	}
	return nil
}

// Provider constructs the provider for the supplied family credentials.
func (f apiFamily) Provider(args providerhost.FamilyArgs) (provider.Provider, error) {
	settings, ok := args.Credential.(*providerhost.APIKeyArgs)
	if !ok {
		return nil, providerhost.ErrWrongCredential
	}
	switch f.name {
	case "anthropic":
		return anthropicapi.New(
			anthropicapi.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL, Policy: settings.Vendor},
			args.Clock,
		), nil
	case "google":
		return google.New(google.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL}, args.Clock), nil
	default:
		wire := types.WireAPIResponses
		if settings.WireAPI != nil {
			wire = *settings.WireAPI
		}
		return openaiapi.New(
			openaiapi.Config{APIKey: settings.APIKey, BaseURL: settings.BaseURL, Wire: wire, Policy: settings.Vendor},
			args.Clock,
		), nil
	}
}

type subscriptionFamily struct{ name string }

// Credential identifies the credentials this provider family accepts.
func (subscriptionFamily) Credential() webapiproto.CredentialKind {
	return webapiproto.CredentialKindSubscription
}

// Wires lists the selectable wire protocols for this family.
func (subscriptionFamily) Wires() []types.WireAPI { return nil }

// Provider constructs the provider for the supplied family credentials.
func (f subscriptionFamily) Provider(args providerhost.FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(*providerhost.SubscriptionArgs)
	if !ok {
		return nil, providerhost.ErrWrongCredential
	}
	account, quota := boundAccount(subscription.Account)
	if f.name == "codex" {
		return codex.New(codex.Config{Account: account}, subscription.Pool, quota, args.HTTP, args.Clock)
	}
	return grokbuild.New(grokbuild.Config{Account: account}, subscription.Pool, quota, args.HTTP, args.Clock), nil
}

type claudeFamily struct{}

// Credential identifies the credentials this provider family accepts.
func (claudeFamily) Credential() webapiproto.CredentialKind {
	return webapiproto.CredentialKindSubscription
}

// Wires lists the selectable wire protocols for this family.
func (claudeFamily) Wires() []types.WireAPI { return nil }

// Provider constructs the provider for the supplied family credentials.
func (f claudeFamily) Provider(args providerhost.FamilyArgs) (provider.Provider, error) {
	return f.provider(args)
}

// ProcessRuntime constructs a Claude Code runtime for the supplied placement.
func (f claudeFamily) ProcessRuntime(
	_ context.Context,
	args providerhost.FamilyArgs,
	placement claudecode.Placement,
) (provider.Runtime, error) {
	p, err := f.provider(args)
	if err != nil {
		return nil, err
	}
	return p.ProcessRuntime(placement), nil
}

func (claudeFamily) provider(args providerhost.FamilyArgs) (*claudecode.Provider, error) {
	subscription, ok := args.Credential.(*providerhost.SubscriptionArgs)
	if !ok {
		return nil, providerhost.ErrWrongCredential
	}
	account, quota := boundAccount(subscription.Account)
	return claudecode.New(
		claudecode.NewConfig(args.EntryID, args.Label, account),
		subscription.Pool,
		quota,
		args.ModelsDev,
		args.HTTP,
		args.Clock,
	), nil
}

// boundAccount supplies the account's quota, or an unwritten store for login.
func boundAccount(account *providerhost.AccountBinding) (*string, provider.QuotaSnapshotStore) {
	if account == nil {
		return nil, &provider.MemorySnapshots{}
	}
	return &account.CredentialID, account.Quota
}

// BuiltinPlugins returns the plugins built into the backend in registration
// order: file, todo, browser, expose, skills, changes, file browser.
// Declaration failures are returned instead of panicking.
func BuiltinPlugins() ([]plugin.Factory, error) {
	files, err := file.New()
	if err != nil {
		return nil, err
	}
	todos, err := todo.New()
	if err != nil {
		return nil, err
	}
	browsers, err := browser.New()
	if err != nil {
		return nil, err
	}
	exposes, err := expose.New()
	if err != nil {
		return nil, err
	}
	skill, err := skills.New()
	if err != nil {
		return nil, err
	}
	return []plugin.Factory{files, todos, browsers, exposes, skill, changes.New(), filebrowser.New()}, nil
}
