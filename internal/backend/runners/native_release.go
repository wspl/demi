package runners

import (
	"context"
	"errors"
	"fmt"
	"maps"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandproto"
)

type verifiedRelease struct {
	descriptor            commandproto.PackageDescriptor
	executables, archives []ArtifactFile
}

// verifyReleases checks every native release before any bytes can be published.
func verifyReleases(ctx context.Context, releases []NativeRelease, allTargets bool) ([]verifiedRelease, error) {
	verified := make([]verifiedRelease, 0, len(releases))
	ids := make(map[string]bool)
	for _, release := range releases {
		checked, err := verifyRelease(ctx, release, allTargets)
		if err != nil {
			return nil, err
		}
		if ids[checked.descriptor.ID] {
			return nil, configError(fmt.Errorf("%s is released twice", checked.descriptor.ID))
		}
		ids[checked.descriptor.ID] = true
		verified = append(verified, checked)
	}
	return verified, nil
}

// verifyRelease checks each declared executable and resource archive by size and hash.
func verifyRelease(ctx context.Context, release NativeRelease, allTargets bool) (verifiedRelease, error) {
	data, err := os.ReadFile(filepath.Join(release.Directory, "descriptor.json"))
	if err != nil {
		return verifiedRelease{}, releaseError(release.Directory, fmt.Errorf("descriptor.json: %w", err))
	}
	descriptor, err := commandproto.DecodePackageDescriptor(data)
	if err != nil {
		return verifiedRelease{}, releaseError(release.Directory, fmt.Errorf("descriptor.json: %w", err))
	}
	carried := commandproto.Targets
	if !allTargets {
		carried = slices.Sorted(maps.Keys(descriptor.Targets))
	}
	if len(carried) == 0 {
		return verifiedRelease{}, releaseError(release.Directory, errors.New("it carries no target"))
	}
	verified := verifiedRelease{descriptor: descriptor}
	for _, target := range carried {
		expected, exists := descriptor.Targets[target]
		if !exists {
			return verifiedRelease{}, releaseError(release.Directory, errors.New("it lacks a target: "+target))
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
			return verifiedRelease{}, releaseError(release.Directory, fmt.Errorf("%s: %w", target, err))
		}
		if found.Size != expected.Size || found.SHA256 != expected.SHA256 {
			return verifiedRelease{}, releaseError(
				release.Directory,
				errors.New(target+": the executable does not match the descriptor"),
			)
		}
		verified.executables = append(verified.executables, ArtifactFile{Path: path, Artifact: expected})
	}
	archives, err := verifyResources(ctx, release, descriptor)
	if err != nil {
		return verifiedRelease{}, err
	}
	verified.archives = archives
	return verified, nil
}

// verifyResources checks every resource archive in descriptor order before publication.
func verifyResources(
	ctx context.Context,
	release NativeRelease,
	descriptor commandproto.PackageDescriptor,
) ([]ArtifactFile, error) {
	var archives []ArtifactFile
	for _, name := range slices.Sorted(maps.Keys(descriptor.Resources)) {
		resource := descriptor.Resources[name]
		for _, target := range slices.Sorted(maps.Keys(resource.Targets)) {
			archive := resource.Targets[target]
			expected := archive.Archive()
			path := filepath.Join(release.Directory, "resources", expected.SHA256)
			found, err := artifacts.DigestFile(ctx, path, expected.Size)
			if err != nil {
				if ctx.Err() != nil {
					return nil, publicationStoreError(ctx.Err())
				}
				return nil, releaseError(release.Directory, fmt.Errorf("%s for %s: %w", name, target, err))
			}
			if found.Size != expected.Size || found.SHA256 != expected.SHA256 {
				return nil, releaseError(
					release.Directory,
					fmt.Errorf("%s for %s: the archive does not match the descriptor", name, target),
				)
			}
			archives = append(archives, ArtifactFile{Path: path, Artifact: expected})
		}
	}
	return archives, nil
}
