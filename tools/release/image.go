package main

import (
	"archive/tar"
	"compress/gzip"
	"context"
	_ "embed"
	"errors"
	"fmt"
	"io"
	"math"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"slices"
	"strings"

	"github.com/klauspost/compress/zstd"
	"github.com/wspl/demi/internal/artifacts"
	"github.com/wspl/demi/internal/commandproto"
	"github.com/wspl/demi/internal/machinemanagerproto"
	"github.com/wspl/demi/internal/runnerproto"
)

// The authoritative pin remains cloud-guest-image/rootfs/uv.json. This generated
// copy lets the cross-built assembler carry its pin without a source checkout.
//
//go:generate cp ../../cloud-guest-image/rootfs/uv.json uv.json
//go:embed uv.json
var uvPin []byte

// The pinned uv release (`cloud-guest-image/rootfs/uv.json`): its
// version and the archive for each image architecture.
// +demi:root
// +demi:strict
type uvRelease struct {
	// +demi:length chars min=1
	Version string    `json:"version"`
	AMD64   uvArchive `json:"amd64"`
	ARM64   uvArchive `json:"arm64"`
}

// One architecture's uv archive, a gzip-compressed tar file: where it is
// downloaded from, its size and SHA-256, and the paths of its executables,
// each installed in `/usr/local/bin` under its file name.
// +demi:strict
// +demi:check validateUVArchive
type uvArchive struct {
	// +demi:length chars min=1
	URL string `json:"url"`
	// +demi:range min=1
	Size uint64 `json:"size"`
	// +demi:pattern ^[0-9a-f]{64}$
	SHA256 string `json:"sha256"`
	// +demi:length min=1
	Executables []string `json:"executables"`
}

func validateUVArchive(archive uvArchive) error {
	parsed, err := url.Parse(archive.URL)
	if err != nil {
		return fmt.Errorf("uv URL: %w", err)
	}
	if !parsed.IsAbs() {
		return errors.New("uv URL is not absolute")
	}
	for _, name := range archive.Executables {
		if name == "" {
			return errors.New("uv executable path is empty")
		}
	}
	return nil
}

func (a *application) image(ctx context.Context, o imageOptions) error {
	architecture, ok := machinemanagerproto.HostArchitecture()
	if runtime.GOOS != "linux" || !ok {
		return errors.New("a Cloud image is packaged on a Linux builder of its architecture")
	}
	pin, err := decodeUvRelease(uvPin)
	if err != nil {
		return fmt.Errorf("the pinned uv release is invalid: %w", err)
	}
	client := artifacts.NewClient()
	defer client.Close()
	return a.packageImage(ctx, o, architecture, pin, client, a.writeArchive)
}

func (a *application) packageImage(
	ctx context.Context,
	o imageOptions,
	architecture machinemanagerproto.Architecture,
	pin uvRelease,
	client *artifacts.Client,
	writeArchive func(context.Context, string, string) error,
) (err error) {
	root, err := filepath.Abs(o.Root)
	if err != nil {
		return err
	}
	output, err := filepath.Abs(o.Output)
	if err != nil {
		return err
	}
	target := architecture.Target()
	inventory, ubuntu, err := imageSystem(root)
	if err != nil {
		return err
	}
	runner, runnerArtifact, err := imageRunnerRelease(o.Runners, target)
	if err != nil {
		return err
	}
	releases, err := imageCommandReleases(o, target)
	if err != nil {
		return err
	}
	executables := make(map[string]commandproto.PackageArtifact)
	if err := a.installImageRunner(ctx, root, o.Runners, target, runner, runnerArtifact, executables); err != nil {
		return err
	}
	if err := a.installImageCommands(ctx, root, target, o.Packages, releases, executables); err != nil {
		return err
	}
	archive := pin.AMD64
	if architecture == machinemanagerproto.ArchitectureARM64 {
		archive = pin.ARM64
	}
	if err := installUV(ctx, root, archive, client, executables); err != nil {
		return err
	}
	if _, err := fmt.Fprintln(a.Err, "Cloud image: uv", pin.Version); err != nil {
		return err
	}
	init, err := measureExecutable(ctx, inTree(root, machinemanagerproto.InitPath))
	if err != nil {
		return err
	}
	executables[machinemanagerproto.InitPath] = init
	return a.publishImageArchive(
		ctx,
		imageArchiveOptions{root, output, architecture, ubuntu, inventory, executables, releases, runner, archive, pin},
		writeArchive,
	)
}

func inTree(root, path string) string {
	return filepath.Join(root, filepath.FromSlash(strings.TrimLeft(path, "/")))
}

func releaseExecutable(directory, target string) (string, error) {
	directory = filepath.Join(directory, target)
	entries, err := os.ReadDir(directory)
	if err != nil {
		return "", err
	}
	if len(entries) != 1 {
		return "", fmt.Errorf("%s does not hold exactly one executable", directory)
	}
	return filepath.Join(directory, entries[0].Name()), nil
}

func installExecutable(
	ctx context.Context,
	source, destination string,
	artifact commandproto.PackageArtifact,
) (err error) {
	input, err := os.Open(source)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, input.Close())
	}()
	stage, err := artifacts.NewStaged(ctx, destination, artifacts.Publication{Permissions: artifacts.Executable})
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, stage.Close())
	}()
	if err := artifacts.Copy(
		ctx,
		input,
		artifacts.Digest{Size: artifact.Size, SHA256: artifact.SHA256},
		stage.File(),
	); err != nil {
		return fmt.Errorf("%s: %w", source, err)
	}
	return stage.Publish(ctx)
}

func measureExecutable(ctx context.Context, path string) (commandproto.PackageArtifact, error) {
	info, err := os.Lstat(path)
	if err != nil {
		return commandproto.PackageArtifact{}, err
	}
	if !info.Mode().IsRegular() {
		return commandproto.PackageArtifact{}, fmt.Errorf("%s is not a regular file", path)
	}
	digest, err := artifacts.DigestFile(ctx, path, math.MaxUint64)
	return commandproto.PackageArtifact{SHA256: digest.SHA256, Size: digest.Size}, err
}

func (a *application) installResources(
	ctx context.Context,
	root, directory, target string,
	descriptor commandproto.PackageDescriptor,
	executables map[string]commandproto.PackageArtifact,
) error {
	names := make([]string, 0, len(descriptor.Resources))
	for name := range descriptor.Resources {
		names = append(names, name)
	}
	slices.Sort(names)
	for _, name := range names {
		resource := descriptor.Resources[name]
		archive, ok := resource.Targets[target]
		if !ok {
			continue
		}
		entry, err := installResource(
			ctx,
			inTree(root, runnerproto.ArtifactsPath),
			filepath.Join(directory, "resources", archive.SHA256),
			archive,
		)
		if err != nil {
			return err
		}
		digest, err := measureExecutable(ctx, entry)
		if err != nil {
			return err
		}
		executables[runnerproto.ArtifactsPath+"/"+archive.SHA256+"/"+archive.Entry] = digest
		if _, err := fmt.Fprintf(a.Err, "Cloud image: %s for %s\n", resource.Title, descriptor.ID); err != nil {
			return err
		}
	}
	return nil
}

func installResource(
	ctx context.Context,
	root, source string,
	archive commandproto.ResourceArtifact,
) (entry string, err error) {
	_, unpacking, err := artifacts.InstallArchive(
		ctx,
		root,
		artifacts.Archive{Digest: artifacts.Digest{Size: archive.Size, SHA256: archive.SHA256}, Entry: archive.Entry},
	)
	if err != nil {
		return "", err
	}
	if unpacking == nil {
		return "", fmt.Errorf("%s is in the image twice", source)
	}
	defer func() {
		err = errors.Join(err, unpacking.Close())
	}()
	input, err := os.Open(source)
	if err != nil {
		return "", err
	}
	defer func() {
		err = errors.Join(err, input.Close())
	}()
	output, err := os.Create(unpacking.ArchivePath())
	if err != nil {
		return "", err
	}
	copyErr := artifacts.Copy(ctx, input, artifacts.Digest{Size: archive.Size, SHA256: archive.SHA256}, output)
	if err := errors.Join(copyErr, output.Close()); err != nil {
		return "", err
	}
	entry, err = unpacking.Finish(ctx)
	if err != nil {
		return "", err
	}
	// Finish releases the lock; no concurrent installer runs in an image tree.
	if err := unpacking.Close(); err != nil {
		return "", err
	}
	if err := os.Remove(filepath.Join(root, archive.SHA256+".lock")); err != nil {
		return "", err
	}
	return entry, nil
}

func installUV(
	ctx context.Context,
	root string,
	archive uvArchive,
	client *artifacts.Client,
	executables map[string]commandproto.PackageArtifact,
) (err error) {
	scratch, err := os.MkdirTemp("", "demi-uv-")
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, os.RemoveAll(scratch))
	}()
	path := filepath.Join(scratch, "uv.tar.gz")
	if err := downloadArchive(
		ctx,
		client,
		archive.URL,
		path,
		artifacts.Digest{Size: archive.Size, SHA256: archive.SHA256},
	); err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, file.Close())
	}()
	zipped, err := gzip.NewReader(file)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, zipped.Close())
	}()
	return installUVExecutables(ctx, root, archive, tar.NewReader(zipped), executables)
}

func installedPackages(root string) ([]machinemanagerproto.InstalledPackage, error) {
	path := inTree(root, "/var/lib/dpkg/status")
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	packages := []machinemanagerproto.InstalledPackage{}
	var name, status, version *string
	for _, line := range append(strings.Split(string(data), "\n"), "") {
		if strings.TrimSpace(line) == "" {
			if name == nil && status == nil && version == nil {
				continue
			}
			var err error
			packages, err = appendInstalledPackage(packages, path, name, status, version)
			if err != nil {
				return nil, err
			}
			name = nil
			status = nil
			version = nil
			continue
		}
		if strings.HasPrefix(line, " ") || strings.HasPrefix(line, "\t") {
			continue
		}
		key, value, ok := strings.Cut(line, ":")
		if !ok {
			return nil, fmt.Errorf("%s has a line without a field: %s", path, line)
		}
		value = strings.TrimSpace(value)
		switch key {
		case "Package":
			name = &value
		case "Status":
			status = &value
		case "Version":
			version = &value
		}
	}
	return packages, nil
}

func (a *application) writeArchive(ctx context.Context, root, path string) (err error) {
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0o666)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, file.Close())
	}()
	encoder, err := zstd.NewWriter(file)
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, encoder.Close())
	}()
	cmd := exec.CommandContext(ctx, "tar", "--numeric-owner", "--xattrs", "--acls", "-cf", "-", "-C", root, ".")
	cmd.Stdout = encoder
	cmd.Stderr = a.Err
	if err := cmd.Run(); err != nil {
		return fmt.Errorf("tar failed: %w", err)
	}
	return nil
}

func imageCommandReleases(o imageOptions, target string) ([]commandproto.PackageDescriptor, error) {
	releases := make([]commandproto.PackageDescriptor, 0, len(o.Packages))
	seen := map[string]bool{}
	for _, directory := range o.Packages {
		data, err := os.ReadFile(filepath.Join(directory, "descriptor.json"))
		if err != nil {
			return nil, err
		}
		descriptor, err := commandproto.DecodePackageDescriptor(data)
		if err != nil {
			return nil, err
		}
		if _, ok := descriptor.Targets[target]; !ok {
			return nil, fmt.Errorf(
				"the command package %s@%s carries nothing for %s",
				descriptor.ID,
				descriptor.Version,
				target,
			)
		}
		if seen[descriptor.ID] {
			return nil, fmt.Errorf("the command package %s is named twice", descriptor.ID)
		}
		seen[descriptor.ID] = true
		releases = append(releases, descriptor)
	}

	return releases, nil
}

func installUVExecutables(
	ctx context.Context,
	root string,
	archive uvArchive,
	reader *tar.Reader,
	executables map[string]commandproto.PackageArtifact,
) error {
	found := map[string]bool{}
	for {
		if err := ctx.Err(); err != nil {
			return err
		}
		header, err := reader.Next()
		if errors.Is(err, io.EOF) {
			break
		}
		if err != nil {
			return err
		}
		if !slices.Contains(archive.Executables, header.Name) {
			continue
		}
		if header.Typeflag != tar.TypeReg {
			return fmt.Errorf("the uv archive %s holds %s as something other than a file", archive.URL, header.Name)
		}
		name := filepath.Base(header.Name)
		path := "/usr/local/bin/" + name
		if err := artifacts.Publish(
			ctx,
			inTree(root, path),
			reader,
			artifacts.Publication{Permissions: artifacts.Executable},
		); err != nil {
			return err
		}
		artifact, err := measureExecutable(ctx, inTree(root, path))
		if err != nil {
			return err
		}
		executables[path] = artifact
		found[header.Name] = true
	}
	for _, name := range archive.Executables {
		if !found[name] {
			return fmt.Errorf("the uv archive %s holds no %s", archive.URL, name)
		}
	}
	return nil
}

func appendInstalledPackage(
	packages []machinemanagerproto.InstalledPackage,
	path string,
	name, status, version *string,
) ([]machinemanagerproto.InstalledPackage, error) {
	if name == nil || status == nil {
		return nil, fmt.Errorf("%s lists a package without its name or status", path)
	}
	words := strings.Fields(*status)
	if len(words) != 3 || words[1] != "ok" ||
		!slices.Contains([]string{"installed", "not-installed", "config-files"}, words[2]) {
		return nil, fmt.Errorf("dpkg lists %s as %q: its installation did not finish", *name, *status)
	}
	if words[2] == "installed" {
		if version == nil {
			return nil, fmt.Errorf("%s lists %s without its version", path, *name)
		}
		packages = append(packages, machinemanagerproto.InstalledPackage{Name: *name, Version: *version})
	}
	return packages, nil
}

func (a *application) installImageCommands(
	ctx context.Context,
	root, target string,
	directories []string,
	releases []commandproto.PackageDescriptor,
	executables map[string]commandproto.PackageArtifact,
) error {
	for i, descriptor := range releases {
		source, err := releaseExecutable(directories[i], target)
		if err != nil {
			return err
		}
		artifact := descriptor.Targets[target]
		path := runnerproto.ArtifactsPath + "/" + artifact.SHA256 + "/" + filepath.Base(source)
		if err := os.MkdirAll(filepath.Dir(inTree(root, path)), 0o755); err != nil {
			return err
		}
		if err := installExecutable(ctx, source, inTree(root, path), artifact); err != nil {
			return err
		}
		executables[path] = artifact
		if _, err := fmt.Fprintf(
			a.Err,
			"Cloud image: command package %s@%s\n",
			descriptor.ID,
			descriptor.Version,
		); err != nil {
			return err
		}
		if err := a.installResources(ctx, root, directories[i], target, descriptor, executables); err != nil {
			return err
		}
	}

	return nil
}

func ubuntuVersion(osRelease []byte) (string, error) {
	ubuntu := ""
	for line := range strings.SplitSeq(string(osRelease), "\n") {
		if value, ok := strings.CutPrefix(line, "VERSION_ID="); ok {
			ubuntu = strings.Trim(value, "\"'")
			break
		}
	}
	if ubuntu == "" {
		return "", errors.New("os-release names no VERSION_ID")
	}

	return ubuntu, nil
}

func (a *application) publishImageArchive(
	ctx context.Context,
	o imageArchiveOptions,
	writeArchive func(context.Context, string, string) error,
) (err error) {
	parent := filepath.Dir(o.output)
	if err := os.MkdirAll(parent, 0o755); err != nil {
		return err
	}
	scratch, err := os.MkdirTemp(parent, ".cloud-image-")
	if err != nil {
		return err
	}
	defer func() {
		err = errors.Join(err, os.RemoveAll(scratch))
	}()
	archivePath := filepath.Join(scratch, string(machinemanagerproto.RootfsTarZst))
	if _, err := fmt.Fprintln(a.Err, "Cloud image: writing", machinemanagerproto.RootfsTarZst); err != nil {
		return err
	}
	if err := writeArchive(ctx, o.root, archivePath); err != nil {
		return err
	}
	digest, err := artifacts.DigestFile(ctx, archivePath, math.MaxUint64)
	if err != nil {
		return err
	}
	manifest := machinemanagerproto.CloudImageManifest{
		FormatVersion: 1,
		OS:            machinemanagerproto.OSLinux,
		Architecture:  o.architecture,
		Rootfs: machinemanagerproto.RootfsArchive{
			SHA256: digest.SHA256,
			Size:   digest.Size,
			File:   machinemanagerproto.RootfsTarZst,
		},
		Ubuntu:      o.ubuntu,
		Packages:    o.inventory,
		Executables: o.executables,
		Releases:    o.releases,
		Runner:      o.runner,
		Tools: []machinemanagerproto.StandaloneTool{
			{Name: "uv", Version: o.pin.Version, SHA256: o.archive.SHA256},
		},
	}
	return a.publishImageRecord(ctx, o.output, manifest, archivePath, digest)
}

type imageArchiveOptions struct {
	root, output string
	architecture machinemanagerproto.Architecture
	ubuntu       string
	inventory    []machinemanagerproto.InstalledPackage
	executables  map[string]commandproto.PackageArtifact
	releases     []commandproto.PackageDescriptor
	runner       runnerproto.Release
	archive      uvArchive
	pin          uvRelease
}

func imageSystem(root string) ([]machinemanagerproto.InstalledPackage, string, error) {
	inventory, err := installedPackages(root)
	if err != nil {
		return nil, "", err
	}
	osRelease, err := os.ReadFile(filepath.Join(root, "usr/lib/os-release"))
	if err != nil {
		return nil, "", err
	}
	ubuntu, err := ubuntuVersion(osRelease)
	if err != nil {
		return nil, "", err
	}

	return inventory, ubuntu, nil
}

func (a *application) installImageRunner(
	ctx context.Context,
	root, runners, target string,
	runner runnerproto.Release,
	runnerArtifact commandproto.PackageArtifact,
	executables map[string]commandproto.PackageArtifact,
) error {
	source, err := releaseExecutable(filepath.Join(runners, runner.Release), target)
	if err != nil {
		return err
	}
	if err := installExecutable(ctx, source, inTree(root, machinemanagerproto.RunnerPath), runnerArtifact); err != nil {
		return err
	}
	if err := os.Symlink("demi-runner", inTree(root, "/usr/bin/demi")); err != nil {
		return err
	}
	executables[machinemanagerproto.RunnerPath] = runnerArtifact
	if _, err := fmt.Fprintln(a.Err, "Cloud image: runner release", runner.Release); err != nil {
		return err
	}

	return nil
}

func (a *application) publishImageRecord(
	ctx context.Context,
	output string,
	manifest machinemanagerproto.CloudImageManifest,
	archivePath string,
	digest artifacts.Digest,
) error {
	data, err := record(manifest)
	if err != nil {
		return err
	}
	if _, err := machinemanagerproto.DecodeCloudImageManifest(data); err != nil {
		return err
	}
	if err := artifacts.PublishRelease(
		ctx,
		output,
		artifacts.ReleaseRecord{Name: "manifest.json", Bytes: data},
		[]artifacts.ReleaseFile{{Source: archivePath, Path: string(machinemanagerproto.RootfsTarZst), Digest: digest}},
	); err != nil {
		return err
	}
	if _, err := fmt.Fprintln(a.Err, "Cloud image:", output); err != nil {
		return err
	}
	digest, err = artifacts.DigestFile(ctx, filepath.Join(output, "manifest.json"), math.MaxUint64)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintln(a.Out, digest.SHA256)
	return err
}

func imageRunnerRelease(runners, target string) (runnerproto.Release, commandproto.PackageArtifact, error) {
	data, err := os.ReadFile(filepath.Join(runners, "manifest.json"))
	if err != nil {
		return runnerproto.Release{}, commandproto.PackageArtifact{}, err
	}
	runner, err := runnerproto.DecodeRelease(data)
	if err != nil {
		return runnerproto.Release{}, commandproto.PackageArtifact{}, err
	}
	runnerArtifact, ok := runner.Targets[target]
	if !ok {
		return runnerproto.Release{}, commandproto.PackageArtifact{}, fmt.Errorf(
			"the runner release %s carries nothing for %s",
			runner.Release,
			target,
		)
	}

	return runner, runnerArtifact, nil
}
