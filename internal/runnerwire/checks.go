package runnerwire

import (
	"fmt"
	"strings"

	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
)

// validateInstall prevents reported progress from passing the artifact size.
func validateInstall(install Install) error {
	// Rust's unqualified garde length counts UTF-8 bytes, not characters.
	for _, field := range []struct {
		name, value string
		max         int
	}{
		{"package", install.Package, 200},
		{"name", install.Name, 100},
		{"version", install.Version, 100},
	} {
		if len(field.value) > field.max {
			return contract.At(field.name, fmt.Errorf("string length outside bounds"))
		}
	}

	if install.Done > install.Total {
		return fmt.Errorf("is past the total")
	}
	return nil
}

// validateJobFileChange prevents a file path from containing NUL.
func validateJobFileChange(change JobFileChange) error {
	if strings.ContainsRune(change.Path, 0) {
		return fmt.Errorf("path contains NUL")
	}
	return nil
}

// validateServiceOpen requires operation arguments to be a JSON object.
func validateServiceOpen(message ServiceOpen) error {
	if message.Args == nil {
		return nil
	}
	_, err := contract.Object(*message.Args)
	return contract.At("args", err)
}

// validateRPCCall requires command arguments to be a JSON object.
func validateRPCCall(message RPCCall) error {
	_, err := contract.Object(message.Args)
	return contract.At("args", err)
}

// validateArtifactResolve checks the native target's release name.
func validateArtifactResolve(message ArtifactResolve) error {
	if !commandwire.IsTarget(message.Target) {
		return fmt.Errorf("invalid target: %s", message.Target)
	}
	return nil
}

// validateRunnerInfo checks the runner's native target when supplied.
func validateRunnerInfo(info RunnerInfo) error {
	if info.NativeTarget != nil && !commandwire.IsTarget(*info.NativeTarget) {
		return fmt.Errorf("invalid target: %s", *info.NativeTarget)
	}
	return nil
}
