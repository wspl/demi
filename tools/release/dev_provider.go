package main

import (
	"fmt"
	"strconv"
	"strings"

	"github.com/wspl/demi/internal/webapiproto"
)

// devProvider is a real model the development backend seeds beside Echo, from
// the DEMI_DEV_PROVIDER_* variables (backend.md § One-command development backend).
type devProvider struct {
	BaseURL       webapiproto.EndpointURL
	APIKey        string
	Model         string
	ContextWindow uint32
}

// readDevProvider validates the optional development model before any builds.
func readDevProvider(getenv func(string) string) (devProvider, bool, error) {
	names := []string{
		"DEMI_DEV_PROVIDER_BASE_URL",
		"DEMI_DEV_PROVIDER_API_KEY",
		"DEMI_DEV_PROVIDER_MODEL",
		"DEMI_DEV_PROVIDER_CONTEXT_WINDOW",
	}
	values := make([]string, len(names))
	configured := false
	for i, name := range names {
		values[i] = getenv(name)
		configured = configured || values[i] != ""
	}
	if !configured {
		return devProvider{}, false, nil
	}
	for i, value := range values {
		if value == "" {
			return devProvider{}, false, fmt.Errorf(
				"%s is not set; the development provider needs all of %s and %s",
				names[i], strings.Join(names[:len(names)-1], ", "), names[len(names)-1],
			)
		}
	}
	endpoint, err := webapiproto.ParseEndpointURL(values[0])
	if err != nil {
		return devProvider{}, false, fmt.Errorf("%s: %w", names[0], err)
	}
	window, err := strconv.ParseUint(values[3], 10, 32)
	if err != nil {
		return devProvider{}, false, fmt.Errorf("%s must be a positive uint32 integer: %w", names[3], err)
	}
	if window == 0 {
		return devProvider{}, false, fmt.Errorf("%s must be a positive uint32 integer", names[3])
	}
	return devProvider{
		BaseURL: endpoint, APIKey: values[1], Model: values[2], ContextWindow: uint32(window),
	}, true, nil
}
