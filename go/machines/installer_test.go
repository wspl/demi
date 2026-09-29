//go:build linux

package machines_test

import (
	"io"
	"net/netip"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"

	"github.com/wspl/demi/go/machines/internal/config"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/roottest"
)

// The installer's unit and settings (crates/machines/scripts/install-managed-hosts.sh),
// written beneath a staging root: systemd accepts the unit, and the settings
// configure this manager.
func TestTheInstallerWritesAUnitSystemdAcceptsAndSettingsThisManagerReads(t *testing.T) {
	roottest.Require(t)
	if _, err := exec.LookPath("systemd-analyze"); err != nil {
		t.Skip(err)
	}
	directory := t.TempDir()
	// The state directory lives on its own ext4 filesystem.
	image := filepath.Join(directory, "data.img")
	if output, err := exec.Command("mkfs.ext4", "-q", "-F", image, "64m").CombinedOutput(); err != nil {
		t.Skipf("mkfs.ext4: %v\n%s", err, output)
	}
	data := filepath.Join(directory, "data")
	if err := os.Mkdir(data, 0o755); err != nil {
		t.Fatal(err)
	}
	device, err := linux.AttachLoop(image)
	if err != nil {
		t.Fatal(err)
	}
	if err := linux.MountExt4(device.Path(), data); err != nil {
		t.Fatal(err)
	}
	device.Close()
	t.Cleanup(func() { _ = linux.Unmount(data) })
	release := filepath.Join(directory, "image")
	if err := os.Mkdir(release, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(release, "manifest.json"), []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}
	program := filepath.Join(directory, "demi-machines")
	if err := os.WriteFile(program, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(directory, "root")
	script := "../../crates/machines/scripts/install-managed-hosts.sh"
	installed, err := exec.Command("bash", script, "--root", root, "--user", "root", "--manager", program,
		"--image", release, "--backend-url", "https://backend.example.com", "--dns", "1.1.1.1,8.8.8.8",
		"--data", data, "--slots", "16", "--limits", "off").CombinedOutput()
	if err != nil {
		t.Fatalf("the installer: %v\n%s", err, installed)
	}

	unitPath := filepath.Join(root, "etc/systemd/system/demi-machines.service")
	unit, err := os.ReadFile(unitPath)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(string(unit), "\n")
	for _, directive := range []string{
		"Type=notify",
		"KillMode=mixed",
		"TimeoutStartSec=infinity",
		"TimeoutStopSec=infinity",
		"PrivateMounts=yes",
		"UMask=0077",
		"Group=demi-cloud",
		"EnvironmentFile=/etc/demi-machines/manager.env",
		"ExecStart=" + program,
		"ExecStopPost=" + program + " --recover",
	} {
		if !slices.Contains(lines, directive) {
			t.Errorf("%s is missing:\n%s", directive, unit)
		}
	}
	verified, err := exec.Command("systemd-analyze", "verify", unitPath).CombinedOutput()
	if err != nil || strings.Contains(string(verified), "demi-machines.service") {
		t.Errorf("systemd-analyze: %v\n%s", err, verified)
	}

	// Each setting is one this manager knows, and together they configure it.
	settings, err := os.ReadFile(filepath.Join(root, "etc/demi-machines/manager.env"))
	if err != nil {
		t.Fatal(err)
	}
	var environ []string
	for _, line := range strings.Split(strings.TrimSpace(string(settings)), "\n") {
		if !strings.Contains(line, "=") {
			t.Fatalf("a setting is NAME=VALUE: %q", line)
		}
		environ = append(environ, line)
	}
	cfg, err := config.Parse(nil, environ, io.Discard)
	if err != nil {
		t.Fatalf("the installed settings: %v", err)
	}
	if cfg.Mode != config.Serve || cfg.Data != data || cfg.Image != release || cfg.Socket != "/run/demi-cloud/machines.sock" {
		t.Errorf("mode %v, data %s, image %s, socket %s", cfg.Mode, cfg.Data, cfg.Image, cfg.Socket)
	}
	if cfg.BackendURL.String() != "https://backend.example.com/" || cfg.Slots != 16 || cfg.Limits != nil {
		t.Errorf("backend %s, slots %d, limits %+v", cfg.BackendURL, cfg.Slots, cfg.Limits)
	}
	if want := []netip.Addr{netip.MustParseAddr("1.1.1.1"), netip.MustParseAddr("8.8.8.8")}; !slices.Equal(cfg.DNS, want) {
		t.Errorf("dns %v", cfg.DNS)
	}
	if !strings.HasPrefix(cfg.Runsc, "/opt/gvisor") {
		t.Errorf("runsc %s", cfg.Runsc)
	}
}
