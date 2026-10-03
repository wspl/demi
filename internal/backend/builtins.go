package backend

import (
	"context"

	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/plugins/browser"
	"github.com/wspl/demi/internal/plugins/changes"
	"github.com/wspl/demi/internal/plugins/expose"
	"github.com/wspl/demi/internal/plugins/file"
	"github.com/wspl/demi/internal/plugins/filebrowser"
	"github.com/wspl/demi/internal/plugins/skills"
	"github.com/wspl/demi/internal/plugins/todo"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/providers/anthropicapi"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/providers/codex"
	"github.com/wspl/demi/internal/providers/google"
	"github.com/wspl/demi/internal/providers/grokbuild"
	"github.com/wspl/demi/internal/providers/openaiapi"
	"github.com/wspl/demi/internal/webapi"
)

// BuiltinFamilies returns the families built into the backend. Tests can
// register additional scripted families in the returned registry.
func BuiltinFamilies() *providers.FamilyRegistry {
	registry := &providers.FamilyRegistry{}
	registry.Register("anthropic", apiFamily{name: "anthropic"})
	registry.Register(providers.SetupTokenFamily, claudeFamily{})
	registry.Register("codex", subscriptionFamily{name: "codex"})
	registry.Register("google", apiFamily{name: "google"})
	registry.Register("grok-build", subscriptionFamily{name: "grok-build"})
	registry.Register("openai", apiFamily{name: "openai"})
	return registry
}

type apiFamily struct{ name string }

// Credential identifies the credentials this provider family accepts.
func (apiFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }

// Wires lists the selectable wire protocols for this family.
func (f apiFamily) Wires() []core.WireAPI {
	if f.name == "openai" {
		return []core.WireAPI{core.WireAPIResponses, core.WireAPIChatCompletions}
	}
	return nil
}

// Provider constructs the provider for the supplied family credentials.
func (f apiFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	settings, ok := args.Credential.(*providers.APIKeyArgs)
	if !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
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
		wire := core.WireAPIResponses
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
func (subscriptionFamily) Credential() webapi.CredentialKind {
	return webapi.CredentialKindSubscription
}

// Wires lists the selectable wire protocols for this family.
func (subscriptionFamily) Wires() []core.WireAPI { return nil }

// Provider constructs the provider for the supplied family credentials.
func (f subscriptionFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	subscription, ok := args.Credential.(*providers.SubscriptionArgs)
	if !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
	}
	account, quota := boundAccount(subscription.Account)
	if f.name == "codex" {
		return codex.New(codex.Config{Account: account}, subscription.Pool, quota, args.HTTP, args.Clock)
	}
	return grokbuild.New(grokbuild.Config{Account: account}, subscription.Pool, quota, args.HTTP, args.Clock), nil
}

type claudeFamily struct{}

// Credential identifies the credentials this provider family accepts.
func (claudeFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindSubscription }

// Wires lists the selectable wire protocols for this family.
func (claudeFamily) Wires() []core.WireAPI { return nil }

// Provider constructs the provider for the supplied family credentials.
func (f claudeFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	return f.provider(args)
}

// ProcessRuntime constructs a Claude Code runtime for the supplied placement.
func (f claudeFamily) ProcessRuntime(
	_ context.Context,
	args providers.FamilyArgs,
	placement claudecode.Placement,
) (provider.Runtime, error) {
	p, err := f.provider(args)
	if err != nil {
		return nil, err
	}
	return p.ProcessRuntime(placement), nil
}

func (claudeFamily) provider(args providers.FamilyArgs) (*claudecode.Provider, error) {
	subscription, ok := args.Credential.(*providers.SubscriptionArgs)
	if !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
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
func boundAccount(account *providers.AccountBinding) (*string, provider.QuotaSnapshotStore) {
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
