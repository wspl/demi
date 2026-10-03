package claudecode

import (
	"context"
	"fmt"
	"maps"
	"runtime"
	"slices"

	"github.com/nlnwa/whatwg-url/url"
	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/cmdsdk"
	"github.com/wspl/demi/internal/commandwire"
)

const artifactName = "Claude Code"

type operationError struct {
	code  claudecodeop.ErrorCode
	cause error
}

// Error describes the installation operation failure.
func (e *operationError) Error() string {
	switch e.code {
	case claudecodeop.InvalidRelease:
		return "invalid release record: " + e.cause.Error()
	case claudecodeop.InstallFailed:
		return "Claude Code installation failed: " + e.cause.Error()
	}
	return e.cause.Error()
}

// Unwrap returns the operation failure.
func (e *operationError) Unwrap() error { return e.cause }

func parseRelease(input []byte) (claudecodeop.Release, error) {
	release, err := claudecodeop.DecodeRelease(input)
	if err != nil {
		return release, &operationError{claudecodeop.InvalidRelease, err}
	}
	// Check platform URLs in key order, so the error names the first failing key.
	for _, key := range slices.Sorted(maps.Keys(release.Platforms)) {
		parsed, err := url.Parse(release.Platforms[key].URL)
		if err != nil {
			return release, &operationError{claudecodeop.InvalidRelease, fmt.Errorf("platform %s: url: %w", key, err)}
		}
		if parsed.Scheme() != "https" {
			return release, &operationError{
				claudecodeop.InvalidRelease,
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
			claudecodeop.UnsupportedPlatform,
			fmt.Errorf("Claude Code has no build for this machine (%s %s)", runtime.GOOS, runtime.GOARCH),
		}
	}
	return platform, nil
}

func ensure(
	ctx context.Context,
	artifacts *cmdsdk.Artifacts,
	invocation string,
	release claudecodeop.Release,
) (claudecodeop.Installed, error) {
	platform, err := supportedPlatform()
	if err != nil {
		return claudecodeop.Installed{}, err
	}
	artifact, ok := release.Platforms[platform]
	if !ok {
		//nolint:staticcheck // Claude Code is a product name; user-visible text, kept byte for byte.
		return claudecodeop.Installed{}, &operationError{
			claudecodeop.UnsupportedPlatform,
			fmt.Errorf("Claude Code %s has no build for %s", release.Version, platform),
		}
	}
	path, err := artifacts.Install(ctx, commandwire.ArtifactInstall{
		Invocation: invocation,
		Name:       artifactName,
		Version:    string(release.Version),
		SHA256:     artifact.SHA256,
		Size:       artifact.Size,
		Form:       &commandwire.ArtifactFile{},
	})
	if err != nil {
		return claudecodeop.Installed{}, &operationError{claudecodeop.InstallFailed, err}
	}
	return claudecodeop.Installed{Version: string(release.Version), Path: path}, nil
}

func status(ctx context.Context, artifacts *cmdsdk.Artifacts) (claudecodeop.Status, error) {
	platform, err := supportedPlatform()
	if err != nil {
		return claudecodeop.Status{}, err
	}
	installed, err := artifacts.Installed(ctx, artifactName)
	if err != nil {
		return claudecodeop.Status{}, &operationError{claudecodeop.InstallFailed, err}
	}
	result := claudecodeop.Status{Platform: platform, Installed: make([]claudecodeop.Installed, 0, len(installed))}
	for _, item := range installed {
		result.Installed = append(result.Installed, claudecodeop.Installed{Version: item.Version, Path: item.Path})
	}
	return result, nil
}
