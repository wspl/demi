package usershard

import (
	"context"
	"errors"
	"fmt"
	"log/slog"
	"strings"

	"github.com/wspl/demi/internal/backend/cloud"
	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/backend/runners"
	"github.com/wspl/demi/internal/cmdpkg/claudecode/claudecodeop"
	"github.com/wspl/demi/internal/commandwire"
	"github.com/wspl/demi/internal/contract"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/providers/claudecode"
	"github.com/wspl/demi/internal/webapi"
)

type cloudPlacement struct {
	shard        *Shard
	conversation *webapi.ConversationID
	entry        webapi.ProviderID
}
type cliTarget struct {
	host    *remotehost.Host
	home    string
	context commandwire.CommandContext
}

func (p cloudPlacement) Start(ctx context.Context, spawn func(claudecode.Site) host.SpawnRequest) (*host.StartedProcess, error) {
	access, err := cloud.Access(ctx, p.shard)
	if err != nil {
		return nil, err
	}
	defer access.Release()
	site, err := p.site(ctx, access)
	if err != nil {
		return nil, err
	}
	process, err := access.Host.Process().Spawn(ctx, spawn(site))
	if err != nil {
		return nil, fmt.Errorf("Claude Code could not be started on the Cloud: %w", err) //nolint:staticcheck // Product text is copied verbatim from Rust.
	}
	return process, nil
}

func (p cloudPlacement) site(ctx context.Context, access *cloud.MachineAccess) (claudecode.Site, error) {
	var command commandwire.CommandContext
	var err error
	if p.conversation != nil {
		command, err = runners.CommandContext(ctx, p.shard.Control(), p.shard.user, *p.conversation, &commandwire.UserCaller{})
	} else {
		command, err = runners.ProviderContext(ctx, p.shard.Control(), p.shard.user, p.entry)
	}
	if err != nil {
		return claudecode.Site{}, fmt.Errorf("The command context could not be read: %w", err) //nolint:staticcheck // Product text is copied verbatim from Rust.
	}
	target := cliTarget{host: access.Host, home: access.Home, context: command}
	executable, err := p.shard.cliExecutable(ctx, target)
	if err != nil {
		return claudecode.Site{}, err
	}
	site := claudecode.Site{Executable: executable, RunDir: access.Home + "/.demi/claude/run", ConfigDir: access.Home + "/.demi/claude/config"}
	for _, directory := range []string{site.RunDir, site.ConfigDir} {
		if err := access.Host.FS().Mkdir(ctx, directory, host.MkdirOptions{Recursive: true}); err != nil {
			return site, fmt.Errorf("Claude Code's directory %s could not be made on the Cloud: %w", directory, err) //nolint:staticcheck // Product text is copied verbatim from Rust.
		}
	}
	return site, nil
}

func (s *Shard) cliExecutable(ctx context.Context, target cliTarget) (string, error) {
	wanted, err := s.services.ClaudeReleases.Latest(ctx, false)
	if err != nil {
		return "", fmt.Errorf("Claude Code could not be installed: %w", err) //nolint:staticcheck // Product text is copied verbatim from Rust.
	}
	installed, err := cliInstalled(ctx, s.services, target)
	if err != nil {
		return "", err
	}
	if len(installed) == 0 {
		return cliEnsure(ctx, s.services, target, wanted)
	}
	usable := installed[0]
	for _, candidate := range installed {
		if candidate.Version == string(wanted.Version) {
			usable = candidate
			break
		}
	}
	if usable.Version != string(wanted.Version) {
		s.upgradeCLI(target, wanted)
	}
	return usable.Path, nil
}

func (s *Shard) upgradeCLI(target cliTarget, release claudecodeop.Release) {
	s.mu.Lock()
	if s.closing || s.upgrading[release.Version] {
		s.mu.Unlock()
		return
	}
	if s.upgrading == nil {
		s.upgrading = make(map[claudecodeop.Version]bool)
	}
	s.upgrading[release.Version] = true
	s.work.Add(1)
	s.mu.Unlock()
	go func() {
		defer s.work.Done()
		if _, err := cliEnsure(s.ctx, s.services, target, release); err != nil && s.ctx.Err() == nil {
			slog.Warn("the Cloud's Claude Code was not updated", "user", s.user, "error", err)
		}
		s.mu.Lock()
		delete(s.upgrading, release.Version)
		s.mu.Unlock()
	}()
}

func cliInstalled(ctx context.Context, services *Services, target cliTarget) (installed []claudecodeop.Installed, err error) {
	defer func() {
		if err != nil {
			err = fmt.Errorf("Claude Code could not be installed: %w", err) //nolint:staticcheck // Product text is copied verbatim from Rust.
		}
	}()
	output, exited, err := cliCall(ctx, services, target, claudecodeop.OperationStatus, nil, nil)
	if err != nil {
		return nil, err
	}
	reply, err := claudecodeop.DecodeStatusReply(output)
	if err != nil {
		return nil, fmt.Errorf("the installer's answer cannot be read: %w", err)
	}
	switch reply := reply.(type) {
	case *claudecodeop.Failed:
		return nil, errors.New(reply.Message)
	case *claudecodeop.StatusDone:
		if exited != nil {
			return nil, fmt.Errorf("the installer answered and exited with %d", exited.ExitCode)
		}
		return reply.Installed, nil
	}
	return nil, errors.New("the installer gave no answer")
}

func cliEnsure(ctx context.Context, services *Services, target cliTarget, release claudecodeop.Release) (path string, err error) {
	defer func() {
		if err != nil {
			err = fmt.Errorf("Claude Code %s could not be installed: %w", release.Version, err) //nolint:staticcheck // Product text is copied verbatim from Rust.
		}
	}()
	input, err := contract.EncodeJSON(release)
	if err != nil {
		return "", err
	}
	var attached []remotehost.AttachedArtifact
	for _, artifact := range release.Platforms {
		attached = append(attached, remotehost.AttachedArtifact{Artifact: commandwire.PackageArtifact{SHA256: artifact.SHA256, Size: artifact.Size}, Location: &commandwire.ArtifactURL{URL: artifact.URL}})
	}
	output, exited, err := cliCall(ctx, services, target, claudecodeop.OperationEnsure, input, attached)
	if err != nil {
		return "", err
	}
	reply, err := claudecodeop.DecodeEnsureReply(output)
	if err != nil {
		return "", fmt.Errorf("the installer's answer cannot be read: %w", err)
	}
	switch reply := reply.(type) {
	case *claudecodeop.Failed:
		return "", errors.New(reply.Message)
	case *claudecodeop.Ensured:
		if exited != nil {
			return "", fmt.Errorf("the installer answered and exited with %d", exited.ExitCode)
		}
		return reply.Path, nil
	}
	return "", errors.New("the installer gave no answer")
}

func cliCall(ctx context.Context, services *Services, target cliTarget, operation claudecodeop.Operation, input []byte, attached []remotehost.AttachedArtifact) ([]byte, *remotehost.ServiceCallError, error) {
	pkg := services.Native.Package(claudecodeop.Package)
	if pkg == nil || !services.Native.Serves(claudecodeop.Package, []string{string(claudecodeop.OperationEnsure), string(claudecodeop.OperationStatus)}) {
		return nil, nil, errors.New("this deployment does not carry the demi.claude-code package, which installs it")
	}
	request := remotehost.ServiceRequest{Context: target.context, Package: *pkg, Operation: string(operation), CWD: target.home, Resolver: services.Native.Resolver(services.PublicURL), Attached: attached}
	output, err := target.host.CallService(ctx, request, input, 1024*1024)
	var exited *remotehost.ServiceCallError
	if err != nil {
		if !errors.As(err, &exited) || exited.Kind != remotehost.ServiceExited {
			return nil, nil, fmt.Errorf("the installer failed: %w", err)
		}
		output = exited.Stdout
	}
	if len(strings.TrimSpace(string(output))) == 0 {
		if exited != nil {
			return nil, nil, fmt.Errorf("the installer exited with %d: %s", exited.ExitCode, strings.TrimSpace(exited.Stderr))
		}
		return nil, nil, errors.New("the installer gave no answer")
	}
	return output, exited, nil
}

func (s *Shard) cliMachines(ctx context.Context, entry webapi.ProviderID) ([]webapi.CLIMachine, error) {
	device, err := s.Control().ManagedDevice(ctx, s.user)
	if err != nil {
		return nil, err
	}
	result := []webapi.CLIMachine{}
	if device == nil {
		return result, nil
	}
	remote := s.devices.DeviceAccess(device.ID)
	home, ok := s.devices.Home(device.ID)
	if remote == nil || !ok {
		return result, nil
	}
	command, err := runners.ProviderContext(ctx, s.Control(), s.user, entry)
	if err != nil {
		return nil, err
	}
	var versions *[]string
	installed, err := cliInstalled(ctx, s.services, cliTarget{host: remote, home: home, context: command})
	if err == nil {
		found := make([]string, 0, len(installed))
		for _, version := range installed {
			found = append(found, version.Version)
		}
		versions = &found
	}
	return append(result, webapi.CLIMachine{DeviceID: device.ID, Name: device.Name, Versions: versions}), nil
}

func startInstall(ctx context.Context, services *Services, shards *Shards, user webapi.UserID, entry webapi.ProviderID) (webapi.CLIInstall, error) {
	installs := services.CLIInstalls
	key := installKey{user, entry}
	installs.mu.Lock()
	if _, running := installs.states[key].(*webapi.CLIInstallInstalling); running {
		installs.mu.Unlock()
		return &webapi.CLIInstallInstalling{}, nil
	}
	if installs.states == nil {
		installs.states = make(map[installKey]webapi.CLIInstall)
	}
	installs.states[key] = &webapi.CLIInstallInstalling{}
	installs.mu.Unlock()
	shard, err := shards.Of(ctx, user)
	if err == nil {
		shard.mu.Lock()
		if shard.closing {
			err = &ShardUnavailable{Kind: ShardClosing}
		} else {
			shard.work.Add(1)
		}
		shard.mu.Unlock()
	}
	finish := func(outcome webapi.CLIInstall) {
		installs.mu.Lock()
		if _, exists := installs.states[key]; exists {
			installs.states[key] = outcome
		}
		installs.mu.Unlock()
	}
	if err != nil {
		failed := &webapi.CLIInstallFailed{Message: err.Error()}
		finish(failed)
		return failed, nil
	}
	go func() {
		defer shard.work.Done()
		access, err := cloud.Access(shard.ctx, shard)
		var site claudecode.Site
		if err == nil {
			defer access.Release()
			site, err = (cloudPlacement{shard: shard, entry: entry}).site(shard.ctx, access)
		}
		if shard.ctx.Err() != nil {
			return
		}
		if err != nil {
			finish(&webapi.CLIInstallFailed{Message: err.Error()})
		} else {
			finish(&webapi.CLIInstallInstalled{Path: site.Executable})
		}
	}()
	return &webapi.CLIInstallInstalling{}, nil
}
