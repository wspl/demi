package main

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"slices"
	"strings"

	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandpackage/browser/browserproto"
	"github.com/wspl/demi/internal/commandpackage/claudecode/claudecodeproto"
	"github.com/wspl/demi/internal/commandpackage/file/fileproto"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/runnerproto"
	"github.com/wspl/demi/internal/version"
)

//go:generate go run github.com/wspl/demi/tools/contractgen

var (
	commandPrograms = []string{"demi-file", "demi-browser", "demi-claude-code"}
	defaultPrograms = append([]string{"demi-runner"}, commandPrograms...)
)

func repository() (string, error) {
	dir, err := os.Getwd()
	if err != nil {
		return "", err
	}
	for {
		if _, err := os.Stat(filepath.Join(dir, "go.mod")); err == nil {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", errors.New("cannot find repository go.mod")
		}
		dir = parent
	}
}

func supported(program, target string) bool {
	switch program {
	case "demi-runner", "demi-file", "demi-browser", "demi-claude-code":
		return true
	case "demi-backend":
		return !strings.Contains(target, "windows")
	case "demi-machine-manager":
		return strings.Contains(target, "linux")
	default:
		return false
	}
}

func selectTargets(programs, named []string) ([]string, error) {
	for _, p := range programs {
		if !slices.Contains(append(slices.Clone(defaultPrograms), "demi-backend", "demi-machine-manager"), p) {
			return nil, fmt.Errorf("unknown executable %s", p)
		}
	}
	for _, t := range named {
		if !slices.Contains(commandproto.Targets, t) {
			return nil, fmt.Errorf("the targets are %s", strings.Join(commandproto.Targets, ", "))
		}
		for _, p := range programs {
			if !supported(p, t) {
				return nil, fmt.Errorf("%s is not built for %s", p, t)
			}
		}
	}
	var result []string
	for _, t := range commandproto.Targets {
		if len(named) > 0 && !slices.Contains(named, t) {
			continue
		}
		for _, p := range programs {
			if supported(p, t) {
				result = append(result, t)
				break
			}
		}
	}
	return result, nil
}

func goTarget(target string) (string, string) {
	arch := "amd64"
	if strings.HasPrefix(target, "aarch64-") {
		arch = "arm64"
	}
	platform := "linux"
	if strings.Contains(target, "apple") {
		platform = "darwin"
	}
	if strings.Contains(target, "windows") {
		platform = "windows"
	}
	return platform, arch
}

func executableName(program, target string) string {
	if strings.Contains(target, "windows") {
		return program + ".exe"
	}
	return program
}

func (a *application) directory(named, fallback string) (string, error) {
	if named == "" {
		return filepath.Join(a.Root, fallback), nil
	}
	return filepath.Abs(named)
}

func (a *application) build(ctx context.Context, o buildOptions) error {
	programs := o.Packages
	if len(programs) == 0 {
		programs = defaultPrograms
	}
	targets, err := selectTargets(programs, o.Targets)
	if err != nil {
		return err
	}
	dir, err := a.directory(o.Artifacts, ".cache/native-target")
	if err != nil {
		return err
	}
	for _, target := range targets {
		if _, err := fmt.Fprintln(a.Out, "Native build:", target); err != nil {
			return err
		}
		destination := filepath.Join(dir, target, "release")
		if err := os.MkdirAll(destination, 0o755); err != nil {
			return err
		}
		seen := map[string]bool{}
		for _, p := range programs {
			if !supported(p, target) || seen[p] {
				continue
			}
			seen[p] = true
			if err := a.buildGo(
				ctx,
				target,
				"./cmd/"+p,
				filepath.Join(destination, executableName(p, target)),
				true,
			); err != nil {
				return fmt.Errorf("the build for %s failed: %w", target, err)
			}
		}
	}
	_, err = fmt.Fprintln(a.Out, "Native artifacts:", dir)
	return err
}

// A backend or machine manager release's record: the executable, the
// workspace version it carries, and each target's file.
// +demi:root
type executableRelease struct {
	Executable string                                  `json:"executable"`
	Version    string                                  `json:"version"`
	Targets    map[string]commandproto.PackageArtifact `json:"targets"`
}

// runnerContents is the runner identity's canonical preimage, without the identity.
type runnerContents struct {
	Wire            uint32                                  `json:"wire"`
	CommandProtocol uint64                                  `json:"commandProtocol"`
	Targets         map[string]commandproto.PackageArtifact `json:"targets"`
}

// record writes the release file format with contract.EncodeJSON, keeping its escaping.
func record(value any) ([]byte, error) {
	data, err := contract.EncodeJSON(value)
	if err != nil {
		return nil, err
	}
	var output bytes.Buffer
	if err := json.Indent(&output, data, "", "  "); err != nil {
		return nil, err
	}
	output.WriteByte('\n')
	return output.Bytes(), nil
}

func packageCatalog(program string) (string, []string) {
	switch program {
	case "demi-file":
		return fileproto.Package, fileproto.Operations()
	case "demi-browser":
		return browserproto.Package, browserproto.OperationNames()
	case "demi-claude-code":
		var names []string
		for _, op := range claudecodeproto.Operations() {
			names = append(names, string(op))
		}
		return claudecodeproto.Package, names
	default:
		return "", nil
	}
}

func (a *application) packageNative(ctx context.Context, o packageOptions) error {
	targets, err := selectTargets([]string{o.Package}, o.Targets)
	if err != nil {
		return err
	}
	dir, err := a.directory(o.Artifacts, ".cache/native-target")
	if err != nil {
		return err
	}
	sources := make(map[string]string)
	for _, target := range targets {
		sources[target] = filepath.Join(dir, target, "release", executableName(o.Package, target))
	}
	var chrome browserproto.BrowserRelease
	if o.Package == "demi-browser" {
		chrome, err = browserproto.PinnedRelease()
		if err != nil {
			return err
		}
	}
	return a.publishNative(ctx, o, sources, chrome)
}

func (a *application) publishNative(
	ctx context.Context,
	o packageOptions,
	sources map[string]string,
	chrome browserproto.BrowserRelease,
) error {
	output, err := filepath.Abs(o.Output)
	if err != nil {
		return err
	}
	targets, files, err := nativeReleaseFiles(ctx, o, sources)
	if err != nil {
		return err
	}
	var value any
	name := "release.json"
	directory := output
	message := ""
	switch o.Package {
	case "demi-runner":
		value, directory, message, err = runnerReleaseRecord(output, targets)
		if err != nil {
			return err
		}
		name = "manifest.json"
	case "demi-backend", "demi-machine-manager":
		value = executableRelease{o.Package, version.Release, targets}
		message = fmt.Sprintf("Release %s@%s\n%s", o.Package, version.Release, filepath.Join(output, name))
	default:
		value, name, message, files, err = a.commandRelease(ctx, o, output, chrome, targets, files)
		if err != nil {
			return err
		}
	}
	data, err := record(value)
	if err != nil {
		return err
	}
	if err := artifacts.PublishRelease(
		ctx,
		directory,
		artifacts.ReleaseRecord{Name: name, Bytes: data},
		files,
	); err != nil {
		return err
	}
	if o.Package == "demi-runner" {
		if err := artifacts.PublishBytes(
			ctx,
			filepath.Join(output, name),
			data,
			artifacts.Publication{Mode: artifacts.Replace, Durable: true},
		); err != nil {
			return err
		}
	}
	_, err = fmt.Fprintln(a.Out, message)
	return err
}

func (a *application) browserResources(
	ctx context.Context,
	release browserproto.BrowserRelease,
	selected map[string]commandproto.PackageArtifact,
	cache string,
) (map[string]commandproto.PackageResource, []artifacts.ReleaseFile, error) {
	targets := make(map[string]commandproto.ResourceArtifact)
	var files []artifacts.ReleaseFile
	client := artifacts.NewClient()
	defer client.Close()
	for _, p := range release.Platforms {
		if _, ok := selected[p.Target]; !ok {
			continue
		}
		source := filepath.Join(cache, p.SHA256)
		digest := artifacts.Digest{Size: p.Size, SHA256: p.SHA256}
		if _, err := os.Stat(source); errors.Is(err, os.ErrNotExist) {
			if err := os.MkdirAll(cache, 0o755); err != nil {
				return nil, nil, err
			}
			if _, err := fmt.Fprintln(a.Err, "Downloading", p.URL); err != nil {
				return nil, nil, err
			}
			if err := downloadArchive(ctx, client, p.URL, source, digest); err != nil {
				return nil, nil, err
			}
		} else if err != nil {
			return nil, nil, err
		}
		targets[p.Target] = commandproto.ResourceArtifact{SHA256: p.SHA256, Size: p.Size, Entry: p.Executable}
		files = append(
			files,
			artifacts.ReleaseFile{Source: source, Path: filepath.Join("resources", p.SHA256), Digest: digest},
		)
	}
	if len(targets) == 0 {
		return nil, files, nil
	}
	return map[string]commandproto.PackageResource{
		browserproto.Resource: {Title: release.Title(), Targets: targets},
	}, files, nil
}

func downloadArchive(
	ctx context.Context,
	client *artifacts.Client,
	url, destination string,
	digest artifacts.Digest,
) (err error) {
	stage, err := artifacts.NewStaged(ctx, destination, artifacts.Publication{Mode: artifacts.Replace, Durable: true})
	if err != nil {
		return err
	}
	defer func() { err = errors.Join(err, stage.Close()) }()
	if err := artifacts.Download(ctx, client, url, digest, stage.File()); err != nil {
		return err
	}
	return stage.Publish(ctx)
}

func nativeReleaseFiles(
	ctx context.Context,
	o packageOptions,
	sources map[string]string,
) (map[string]commandproto.PackageArtifact, []artifacts.ReleaseFile, error) {
	targets := make(map[string]commandproto.PackageArtifact)
	var files []artifacts.ReleaseFile
	for _, target := range commandproto.Targets {
		source, ok := sources[target]
		if !ok {
			continue
		}
		digest, err := artifacts.DigestFile(ctx, source, math.MaxUint64)
		if errors.Is(err, os.ErrNotExist) {
			return nil, nil, fmt.Errorf(
				"no build of %s for %s at %s: run go run ./tools/release native build first: %w",
				o.Package,
				target,
				source,
				err,
			)
		}
		if err != nil {
			return nil, nil, fmt.Errorf("%s: %w", source, err)
		}
		targets[target] = commandproto.PackageArtifact{SHA256: digest.SHA256, Size: digest.Size}
		files = append(
			files,
			artifacts.ReleaseFile{
				Source:     source,
				Path:       filepath.Join(target, executableName(o.Package, target)),
				Digest:     digest,
				Executable: true,
			},
		)
	}

	return targets, files, nil
}

func (a *application) commandRelease(
	ctx context.Context,
	o packageOptions,
	output string,
	chrome browserproto.BrowserRelease,
	targets map[string]commandproto.PackageArtifact,
	files []artifacts.ReleaseFile,
) (any, string, string, []artifacts.ReleaseFile, error) {
	var value any
	var name, message string
	id, operations := packageCatalog(o.Package)
	if id == "" {
		return nil, "", "", nil, fmt.Errorf("%s is not a command program", o.Package)
	}
	descriptor := commandproto.PackageDescriptor{
		ID:              id,
		Version:         version.Release,
		ProtocolVersion: commandproto.Version,
		Operations:      operations,
		Targets:         targets,
	}
	if o.Package == "demi-browser" {
		cache, err := a.directory(o.Resources, ".cache/resources")
		if err != nil {
			return nil, "", "", nil, err
		}
		resources, resourceFiles, err := a.browserResources(ctx, chrome, targets, cache)
		if err != nil {
			return nil, "", "", nil, err
		}
		descriptor.Resources = resources
		files = append(files, resourceFiles...)
	}
	if err := descriptor.Validate(); err != nil {
		return nil, "", "", nil, err
	}
	value = descriptor
	name = "descriptor.json"
	digest, err := descriptor.Digest()
	if err != nil {
		return nil, "", "", nil, err
	}
	message = fmt.Sprintf("Command package %s@%s: %s\n%s", id, version.Release, digest, filepath.Join(output, name))
	return value, name, message, files, nil
}

func runnerReleaseRecord(output string, targets map[string]commandproto.PackageArtifact) (any, string, string, error) {
	var value any
	var directory, message string
	contents := runnerContents{runnerproto.Version, commandproto.Version, targets}
	hash, err := commandproto.CanonicalDigest(contents)
	if err != nil {
		return nil, "", "", err
	}
	release := runnerproto.Release{
		Release:         hash,
		Wire:            contents.Wire,
		CommandProtocol: contents.CommandProtocol,
		Targets:         targets,
	}
	if err := release.Validate(); err != nil {
		return nil, "", "", err
	}
	value = release
	directory = filepath.Join(output, hash)
	message = "Runner release: " + directory
	return value, directory, message, nil
}
