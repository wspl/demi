package runners

import (
	"context"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandwire"
)

type verifiedRelease struct {
	descriptor            commandwire.PackageDescriptor
	executables, archives []ArtifactFile
}

// verifyReleases checks every native release before any bytes can be published.
func verifyReleases(ctx context.Context, releases []nativeRelease, allTargets bool) ([]verifiedRelease, error) {
	verified := make([]verifiedRelease, 0, len(releases))
	ids := make(map[string]bool)
	for _, release := range releases {
		checked, err := verifyRelease(ctx, release, allTargets)
		if err != nil {
			return nil, err
		}
		if ids[checked.descriptor.ID] {
			return nil, &PublicationError{Kind: PublicationConfig, Reason: checked.descriptor.ID + " is released twice"}
		}
		ids[checked.descriptor.ID] = true
		verified = append(verified, checked)
	}
	return verified, nil
}

// verifyRelease checks each declared executable and resource archive by size and hash.
func verifyRelease(ctx context.Context, release nativeRelease, allTargets bool) (verifiedRelease, error) {
	refused := func(reason string, cause error) (verifiedRelease, error) {
		return verifiedRelease{}, &PublicationError{Kind: PublicationRelease, Directory: release.Directory, Reason: reason, Err: cause}
	}
	data, err := os.ReadFile(filepath.Join(release.Directory, "descriptor.json"))
	if err != nil {
		return refused("descriptor.json: "+err.Error(), err)
	}
	descriptor, err := commandwire.DecodePackageDescriptor(data)
	if err != nil {
		return refused("descriptor.json: "+err.Error(), err)
	}
	carried := commandwire.Targets
	if !allTargets {
		carried = slices.Sorted(maps.Keys(descriptor.Targets))
	}
	if len(carried) == 0 {
		return refused("it carries no target", nil)
	}
	verified := verifiedRelease{descriptor: descriptor}
	for _, target := range carried {
		expected, exists := descriptor.Targets[target]
		if !exists {
			return refused("it lacks a target: "+target, nil)
		}
		suffix := ""
		if strings.Contains(target, "windows") {
			suffix = ".exe"
		}
		path := filepath.Join(release.Directory, target, release.Executable+suffix)
		found, err := artifacts.DigestFile(ctx, path, expected.Size)
		if err != nil {
			if ctx.Err() != nil {
				return verifiedRelease{}, publicationStoreError(ctx.Err())
			}
			return refused(target+": "+err.Error(), err)
		}
		if found.Size != expected.Size || found.SHA256 != expected.SHA256 {
			return refused(target+": the executable does not match the descriptor", nil)
		}
		verified.executables = append(verified.executables, ArtifactFile{Path: path, Artifact: expected})
	}
	for _, name := range slices.Sorted(maps.Keys(descriptor.Resources)) {
		resource := descriptor.Resources[name]
		for _, target := range slices.Sorted(maps.Keys(resource.Targets)) {
			archive := resource.Targets[target]
			expected := archive.Archive()
			path := filepath.Join(release.Directory, "resources", expected.SHA256)
			found, err := artifacts.DigestFile(ctx, path, expected.Size)
			if err != nil {
				if ctx.Err() != nil {
					return verifiedRelease{}, publicationStoreError(ctx.Err())
				}
				return refused(fmt.Sprintf("%s for %s: %v", name, target, err), err)
			}
			if found.Size != expected.Size || found.SHA256 != expected.SHA256 {
				return refused(fmt.Sprintf("%s for %s: the archive does not match the descriptor", name, target), nil)
			}
			verified.archives = append(verified.archives, ArtifactFile{Path: path, Artifact: expected})
		}
	}
	return verified, nil
}
