package main

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/cookiejar"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/webapi"
)

const devEmail = "developer@example.test"
const devPassword = "development"

func (a *application) dev(ctx context.Context, o devOptions) (err error) {
	if runtime.GOOS == "windows" {
		return errors.New("dev requires Unix process groups")
	}
	programs := filepath.Join(a.Root, ".cache/dev-programs")
	if err := os.MkdirAll(programs, 0755); err != nil {
		return err
	}
	target, err := commandwire.HostTarget()
	if err != nil {
		return err
	}
	for _, name := range append([]string{"demi-backend", "demi-runner", "scripted-machines"}, commandPrograms...) {
		source := "./cmd/" + name
		if name == "scripted-machines" {
			source = "./internal/backend/backendtest/testdata/scripted-machines"
		}
		if err := a.buildGo(ctx, string(target), source, filepath.Join(programs, executableName(name, string(target))), false); err != nil {
			return fmt.Errorf("the workspace build failed: %w", err)
		}
	}

	root, err := os.MkdirTemp("", "demi-dev-")
	if err != nil {
		return err
	}
	defer func() {
		if o.Keep {
			_, writeErr := fmt.Fprintln(a.Out, "Kept the data directory", root)
			err = errors.Join(err, writeErr)
		} else {
			err = errors.Join(err, os.RemoveAll(root))
		}
	}()
	return a.serveDev(ctx, o, root, programs)
}

func devEnvironment() []string {
	var result []string
	for _, entry := range os.Environ() {
		if !strings.HasPrefix(entry, "DEMI_") {
			result = append(result, entry)
		}
	}
	return result
}

// writeDevConfig is blocked on the owner exposing its existing generated native
// configuration contract. Defining another shape here would violate the contract
// boundary. The backend and scripted manager also remain API-checkpoint stubs.
func writeDevConfig(_ context.Context, _ string) (string, error) {
	return "", errors.New("dev requires backend's exported native configuration encoder (internal/backend/runners.nativeConfig is private; request recorded in t-release report)")
}

func seedDev(ctx context.Context, client *http.Client, origin, echo string) (string, error) {
	if _, err := postDev(ctx, client, origin+"/api/setup", webapi.SetupRequest{Email: devEmail, Password: devPassword}); err != nil {
		return "", err
	}
	endpoint, err := webapi.ParseEndpointURL(echo)
	if err != nil {
		return "", err
	}
	models := webapi.ConfiguredModels{{ID: "echo", DisplayName: "Echo", ContextWindow: 200000, ThinkingEfforts: []webapi.ThinkingEffort{}}}
	entry := &webapi.CreateProviderCustom{ProviderType: "anthropic", Label: "Echo", APIKey: "sk-ant-echo", BaseURL: &endpoint, Models: &models}
	data, err := postDev(ctx, client, origin+"/api/providers", entry)
	if err != nil {
		return "", err
	}
	answer, err := webapi.DecodeProviderAnswer(data)
	if err != nil {
		return "", fmt.Errorf("the provider entry's answer: %w", err)
	}
	return string(answer.Provider.ID), nil
}
func postDev(ctx context.Context, client *http.Client, url string, value any) ([]byte, error) {
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return nil, err
	}
	request, err := http.NewRequestWithContext(ctx, http.MethodPost, url, bytes.NewReader(data))
	if err != nil {
		return nil, err
	}
	request.Header.Set("Content-Type", "application/json")
	response, err := client.Do(request)
	if err != nil {
		return nil, err
	}
	defer func() { _ = response.Body.Close() }() // Read-only HTTP body.
	body, err := io.ReadAll(response.Body)
	if err != nil {
		return nil, err
	}
	if response.StatusCode < 200 || response.StatusCode >= 300 {
		return nil, fmt.Errorf("%s answered %s: %s", url, response.Status, body)
	}
	return body, nil
}
func devClient() (*http.Client, error) {
	jar, err := cookiejar.New(nil)
	if err != nil {
		return nil, err
	}
	transport := http.DefaultTransport.(*http.Transport).Clone()
	transport.Proxy = nil
	return &http.Client{Jar: jar, Transport: transport}, nil
}
func answeringDev(ctx context.Context, client *http.Client, origin string, backend *devProcess) error {
	ctx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	for {
		probe, cancel := context.WithTimeout(ctx, time.Second)
		request, err := http.NewRequestWithContext(probe, http.MethodGet, origin+"/api/setup", nil)
		if err != nil {
			cancel()
			return err
		}
		response, err := client.Do(request)
		success := err == nil && response.StatusCode >= 200 && response.StatusCode < 300
		if response != nil {
			_ = response.Body.Close()
		}
		cancel()
		if success {
			return nil
		}
		timer := time.NewTimer(50 * time.Millisecond)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-backend.done:
			timer.Stop()
			return processExit("the backend exited before it answered", backend.err)
		case <-timer.C:
		}
	}
}

// processExit preserves a failed process status and also diagnoses an unexpected
// successful exit, without wrapping a nil error.
func processExit(message string, err error) error {
	if err != nil {
		return fmt.Errorf("%s: %w", message, err)
	}
	return fmt.Errorf("%s: exit status 0", message)
}
