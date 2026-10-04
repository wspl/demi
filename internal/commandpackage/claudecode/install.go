package claudecode

import (
	"context"
	"fmt"
	"maps"
	"runtime"
	"slices"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/commandsdk"
)

const artifactName = "Claude Code"

type operationError struct {
	code  claudecodeproto.ErrorCode
	cause error
}

// Error describes the installation operation failure.
func (e *operationError) Error() string {
	switch e.code {
	case claudecodeproto.InvalidRelease:
		return "invalid release record: " + e.cause.Error()
	case claudecodeproto.InstallFailed:
		return "Claude Code installation failed: " + e.cause.Error()
	}
	return e.cause.Error()
}

// Unwrap returns the operation failure.
func (e *operationError) Unwrap() error {
	return e.cause
}

func parseRelease(input []byte) (claudecodeproto.Release, error) {
	release, err := claudecodeproto.DecodeRelease(input)
	if err != nil {
		return release, &operationError{claudecodeproto.InvalidRelease, err}
	}
	// Check platform URLs in key order, so the error names the first failing key.
	for _, key := range slices.Sorted(maps.Keys(release.Platforms)) {
		parsed, err := url.Parse(release.Platforms[key].URL)
		if err != nil {
			return release, &operationError{
				claudecodeproto.InvalidRelease,
				fmt.Errorf("platform %s: url: %w", key, err),
			}
		}
		if parsed.Scheme() != "https" {
			return release, &operationError{
				claudecodeproto.InvalidRelease,
				fmt.Errorf("platform %s: url must be https", key),
			}
		}
	}
	return release, nil
}

func supportedPlatform() (string, error) {
	platform := currentPlatform()
	if platform == "" {
		//nolint:staticcheck // Claude Code is a product name; user-visible text, kept byte for byte.
		return "", &operationError{
			claudecodeproto.UnsupportedPlatform,
			fmt.Errorf("Claude Code has no build for this machine (%s %s)", runtime.GOOS, runtime.GOARCH),
		}
	}
	return platform, nil
}

func ensure(
	ctx context.Context,
	artifacts *commandsdk.Artifacts,
	invocation string,
	release claudecodeproto.Release,
) (claudecodeproto.Installed, error) {
	platform, err := supportedPlatform()
	if err != nil {
		return claudecodeproto.Installed{}, err
	}
	artifact, ok := release.Platforms[platform]
	if !ok {
		//nolint:staticcheck // Claude Code is a product name; user-visible text, kept byte for byte.
		return claudecodeproto.Installed{}, &operationError{
			claudecodeproto.UnsupportedPlatform,
			fmt.Errorf("Claude Code %s has no build for %s", release.Version, platform),
		}
	}
	path, err := artifacts.Install(ctx, commandproto.ArtifactInstall{
		Invocation: invocation,
		Name:       artifactName,
		Version:    string(release.Version),
		SHA256:     artifact.SHA256,
		Size:       artifact.Size,
		Form:       &commandproto.ArtifactFile{},
	})
	if err != nil {
		return claudecodeproto.Installed{}, &operationError{claudecodeproto.InstallFailed, err}
	}
	return claudecodeproto.Installed{Version: string(release.Version), Path: path}, nil
}

func status(ctx context.Context, artifacts *commandsdk.Artifacts) (claudecodeproto.Status, error) {
	platform, err := supportedPlatform()
	if err != nil {
		return claudecodeproto.Status{}, err
	}
	installed, err := artifacts.Installed(ctx, artifactName)
	if err != nil {
		return claudecodeproto.Status{}, &operationError{claudecodeproto.InstallFailed, err}
	}
	result := claudecodeproto.Status{
		Platform:  platform,
		Installed: make([]claudecodeproto.Installed, 0, len(installed)),
	}
	for _, item := range installed {
		result.Installed = append(result.Installed, claudecodeproto.Installed{Version: item.Version, Path: item.Path})
	}
	return result, nil
}
