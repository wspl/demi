package main

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/provider"
	"github.com/wspl/demi/internal/provider/openaiapi"
	"github.com/wspl/demi/internal/types"
)

func flash(window uint32) types.ModelSelection {
	return types.ModelSelection{
		ProviderID: "deepseek",
		Model: types.Model{
			ID:   environment("DEEPSEEK_FLASH_MODEL", "deepseek-v4-flash"),
			Name: "DeepSeek V4 Flash", ContextWindow: window,
			Thinking: []types.ThinkingCapability{}, AcceptedExtensions: new([]types.FileExtension{}),
		},
	}
}

func deepseek() (*openaiapi.Provider, error) {
	key, ok := os.LookupEnv("DEEPSEEK_API_KEY")
	if !ok {
		return nil, errors.New("DEEPSEEK_API_KEY is not set")
	}
	secret, err := provider.NewSecret(strings.TrimSpace(key))
	if err != nil {
		return nil, err
	}
	base, err := url.Parse(environment("DEEPSEEK_BASE_URL", "https://api.deepseek.com/v1"))
	if err != nil {
		return nil, fmt.Errorf("DEEPSEEK_BASE_URL: %w", err)
	}
	if !base.IsAbs() {
		return nil, errors.New("DEEPSEEK_BASE_URL: relative URL without a base")
	}
	return openaiapi.New(openaiapi.Config{
		APIKey: secret, BaseURL: base, Wire: types.WireAPIChatCompletions,
		Policy: provider.VendorPolicy{PassBackReasoningContent: true},
	}, types.SystemClock{}), nil
}

type deepSeek struct {
	provider  *openaiapi.Provider
	http      *http.Client
	selection types.ModelSelection
}

func (d *deepSeek) Selection(_ context.Context, _ types.NodeID) (types.ModelSelection, error) {
	return d.selection, nil
}

func (d *deepSeek) Runtime(_ context.Context, _ types.NodeID, _ types.ModelSelection) (provider.Runtime, error) {
	return d.provider.Runtime(provider.RuntimeEnv{HTTP: d.http})
}

func environment(variable, fallback string) string {
	if value, ok := os.LookupEnv(variable); ok {
		return value
	}
	return fallback
}

func window(variable string, fallback uint32) (uint32, error) {
	value, ok := os.LookupEnv(variable)
	if !ok {
		return fallback, nil
	}
	parsed, err := strconv.ParseUint(value, 10, 32)
	if err != nil {
		return 0, fmt.Errorf("%s: %w", variable, err)
	}
	return uint32(parsed), nil
}
