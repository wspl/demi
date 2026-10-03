package backendtest

import (
	"context"
	"errors"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/fsnotify/fsnotify"
	"github.com/wspl/demi/internal/backend"
	"github.com/wspl/demi/internal/backend/providers"
	"github.com/wspl/demi/internal/backend/remotehost/remotehosttest"
	"github.com/wspl/demi/internal/core"
	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/providertest"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/webapi"
)

// HostsHarness supplies the scripted machine manager Rust's scenario harness owns.
func HostsHarness(ctx context.Context, t testing.TB) (*Harness, *ScriptedManager, error) {
	t.Helper()
	manager, err := StartScriptedManager(context.WithoutCancel(ctx), t)
	if err != nil {
		return nil, nil, err
	}
	harness, err := NewHarness(ctx, t, manager.Socket())
	return harness, manager, err
}

// HostsReportingStart lets a shutdown-failure scenario observe Close's error once;
// cleanup still closes an unclosed backend, including after a failed assertion.
func HostsReportingStart(
	ctx context.Context,
	t testing.TB,
	h *Harness,
) (*TestBackend, func(context.Context) error, error) {
	b, err := backend.Start(ctx, h.Config)
	if err != nil {
		return nil, nil, err
	}
	client := &http.Client{Transport: http.DefaultTransport.(*http.Transport).Clone()}
	instance := &TestBackend{URL: "http://" + b.LocalAddr().String(), Backend: b, HTTP: client}
	closed := false
	closeReporting := func(ctx context.Context) error {
		closed = true
		return instance.Close(ctx)
	}
	t.Cleanup(func() {
		if !closed {
			if err := closeReporting(context.Background()); err != nil {
				t.Error(err)
			}
		}
	})
	return instance, closeReporting, nil
}

// HostsProcessFamily is the scenarios' API-key family whose runtime needs Cloud
// placement and answers once with "ok", without starting a vendor process.
type HostsProcessFamily struct {
	// Test owns the scripted process provider assertions.
	Test testing.TB
}

// Credential requires an API key.
func (HostsProcessFamily) Credential() webapi.CredentialKind { return webapi.CredentialKindAPIKey }

// Wires has no selectable protocol.
func (HostsProcessFamily) Wires() []core.WireAPI { return nil }

// Provider accepts only the API-key credential.
func (f HostsProcessFamily) Provider(args providers.FamilyArgs) (provider.Provider, error) {
	if _, ok := args.Credential.(*providers.APIKeyArgs); !ok {
		return nil, &providers.FamilyError{Kind: providers.FamilyWrongCredential}
	}
	return hostsProcessProvider{test: f.Test}, nil
}

// ProcessRuntime scripts the runtime over the supplied Cloud placement.
func (f HostsProcessFamily) ProcessRuntime(
	context.Context,
	providers.FamilyArgs,
	claudecode.Placement,
) (provider.Runtime, error) {
	return providertest.NewScriptedRuntime(
		f.Test,
		providertest.Events(providertest.Text("ok"), providertest.Response(1, 1)),
	), nil
}

type hostsProcessProvider struct{ test testing.TB }

// Capabilities returns the fixture provider capabilities.
func (hostsProcessProvider) Capabilities() provider.Capabilities {
	return provider.Capabilities{ProcessHost: true}
}

// AuthStatus reports the fixture authentication state.
func (hostsProcessProvider) AuthStatus(context.Context) core.AuthState { return &core.Authenticated{} }

// RuntimeState reports that the fixture runtime is ready.
func (hostsProcessProvider) RuntimeState() core.RuntimeState { return &core.RuntimeReady{} }

// ListModels returns the scripted provider catalog.
func (hostsProcessProvider) ListModels(context.Context) (core.ProviderModelList, error) {
	return core.ProviderModelList{}, &provider.CatalogError{
		Kind:    provider.CatalogUnavailable,
		Message: "the directory has no answer scripted",
	}
}

// ReadFailure returns empty failure facts for the fixture provider.
func (hostsProcessProvider) ReadFailure(*core.ProviderErrorDiagnostics, core.Timestamp) core.ProviderFailureFacts {
	return core.ProviderFailureFacts{}
}

// Quota returns the fixture quota capability.
func (hostsProcessProvider) Quota() *provider.Quota { return nil }

// Accounts returns the fixture subscription accounts capability.
func (hostsProcessProvider) Accounts() provider.SubscriptionAccounts { return nil }

// Runtime creates the scripted fixture runtime.
func (p hostsProcessProvider) Runtime(provider.RuntimeEnv) (provider.Runtime, error) {
	return providertest.NewScriptedRuntime(
		p.test,
		providertest.Events(providertest.Text("ok"), providertest.Response(1, 1)),
	), nil
}

// HostsInstalledCode waits for the installed runner's console pairing code.
func HostsInstalledCode(ctx context.Context, state string) (code string, err error) {
	watcher, err := fsnotify.NewWatcher()
	if err != nil {
		return "", err
	}
	defer func() { err = errors.Join(err, watcher.Close()) }()
	if err := watcher.Add(state); err != nil {
		return "", err
	}
	for {
		data, err := os.ReadFile(filepath.Join(state, "runner.log"))
		if err != nil && !errors.Is(err, os.ErrNotExist) {
			return "", err
		}
		for _, line := range strings.Split(string(data), "\n") {
			if code, ok := strings.CutPrefix(line, remotehosttest.PairingCode); ok {
				return strings.TrimSpace(code), nil
			}
		}
		select {
		case <-ctx.Done():
			return "", ctx.Err()
		case err := <-watcher.Errors:
			return "", err
		case <-watcher.Events:
		}
	}
}
