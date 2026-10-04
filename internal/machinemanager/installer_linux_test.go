//go:build linux

package machinemanager

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"

	"github.com/wspl/demi/internal/machinemanager/machinemanagertest"
	"github.com/wspl/demi/internal/machinemanager/storage"
	"github.com/wspl/demi/internal/machinemanager/system"
	"github.com/wspl/demi/internal/machinemanager/system/systemtest"
	"github.com/wspl/demi/internal/machinemanagerproto"
)

// Cost: one small ext4 mount and the installer's systemd verification, <1 s normally.
func TestInstallerWritesValidUnitAndSettings(t *testing.T) {
	if !*rootTests {
		t.Skip("requires -machines-root, loop devices and systemd-analyze")
	}
	_, source, _, ok := runtime.Caller(0)
	if !ok {
		t.Fatal("test source path unavailable")
	}
	script := filepath.Join(filepath.Dir(source), "../../scripts/machines/install-managed-hosts.sh")
	directory := t.TempDir()
	architecture, ok := machinemanagerproto.HostArchitecture()
	if !ok {
		t.Fatal("unsupported architecture")
	}
	fixture := machinemanagertest.NewCloudImage(t, machinemanagertest.Entries(), architecture)
	err := systemtest.Isolate(t.Context(), func(ctx context.Context) (err error) {
		tools, err := systemtest.OnPath(ctx)
		if err != nil {
			return err
		}
		image := filepath.Join(directory, "data.img")
		if err = storage.MakeSystem(ctx, tools, image, 64<<20); err != nil {
			return err
		}
		data, release, root := filepath.Join(directory, "data"), fixture.Directory, filepath.Join(directory, "root")
		for _, path := range []string{data} {
			if err = os.Mkdir(path, 0o700); err != nil {
				return err
			}
		}
		device, err := system.Attach(ctx, image)
		if err != nil {
			return err
		}
		defer func() {
			_ = device.Close()
		}()
		if err = system.Ext4(ctx, device.Path(), data); err != nil {
			return err
		}
		defer func() {
			if cleanup := system.Unmount(context.WithoutCancel(ctx), data); err == nil {
				err = cleanup
			}
		}()
		manager := filepath.Join(directory, "demi-machine-manager")
		if err = os.WriteFile(manager, []byte("#!/bin/sh\n"), 0o755); err != nil {
			return err
		}
		command := exec.CommandContext(
			ctx,
			"bash",
			script,
			"--root",
			root,
			"--user",
			"root",
			"--manager",
			manager,
			"--image",
			release,
			"--backend-url",
			"https://backend.example.com",
			"--dns",
			"1.1.1.1,8.8.8.8",
			"--data",
			data,
			"--slots",
			"16",
			"--limits",
			"off",
		)
		if output, err := command.CombinedOutput(); err != nil {
			return fmt.Errorf("installer: %w: %s", err, output)
		}
		unitPath := filepath.Join(root, "etc/systemd/system/demi-machine-manager.service")
		unit, err := os.ReadFile(unitPath)
		if err != nil {
			return err
		}
		for _, directive := range []string{
			"Type=notify",
			"KillMode=mixed",
			"TimeoutStartSec=infinity",
			"TimeoutStopSec=infinity",
			"PrivateMounts=yes",
			"UMask=0077",
			"Group=demi-cloud",
			"EnvironmentFile=/etc/demi-machine-manager/manager.env",
			"ExecStart=" + manager,
			"ExecStopPost=" + manager + " --recover",
		} {
			if !strings.Contains("\n"+string(unit), "\n"+directive+"\n") {
				return fmt.Errorf("unit missing %s", directive)
			}
		}
		output, err := exec.CommandContext(ctx, "systemd-analyze", "verify", unitPath).CombinedOutput()
		if err != nil {
			return fmt.Errorf("verify: %w: %s", err, output)
		}
		if strings.Contains(string(output), "demi-machine-manager.service") {
			return fmt.Errorf("unit warning: %s", output)
		}
		settings, err := os.ReadFile(filepath.Join(root, "etc/demi-machine-manager/manager.env"))
		if err != nil {
			return err
		}
		config, _, err := ParseConfig(nil, strings.Split(strings.TrimSpace(string(settings)), "\n"))
		if err != nil {
			return err
		}
		if config.Mode != ModeServe || config.Data != data || config.Image != release ||
			config.Socket != "/run/demi-cloud/machines.sock" ||
			config.BackendURL.String() != "https://backend.example.com/" ||
			config.Slots != 16 ||
			config.Limits != nil ||
			!strings.HasPrefix(config.Runsc, "/opt/gvisor") ||
			len(config.DNS) != 2 ||
			config.DNS[0].String() != "1.1.1.1" ||
			config.DNS[1].String() != "8.8.8.8" {
			return fmt.Errorf("installed settings: %+v", config)
		}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
}
