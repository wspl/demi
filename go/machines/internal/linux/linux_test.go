//go:build linux

package linux_test

import (
	"errors"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"testing"

	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/roottest"
)

func TestMain(m *testing.M) {
	os.Exit(roottest.Main(m))
}

func makeImage(t *testing.T, path string) {
	t.Helper()
	if output, err := exec.Command("mke2fs", "-q", "-t", "ext4", "-F", path, "32m").CombinedOutput(); err != nil {
		t.Skipf("mke2fs: %v\n%s", err, output)
	}
}

// mountImage makes an ext4 image and mounts it at a new directory of dir. The
// loop device detaches when the filesystem is unmounted.
func mountImage(t *testing.T, dir, name string) (image, target string) {
	t.Helper()
	image = filepath.Join(dir, name+".ext4")
	makeImage(t, image)
	target = filepath.Join(dir, name)
	if err := os.Mkdir(target, 0o755); err != nil {
		t.Fatal(err)
	}
	device, err := linux.AttachLoop(image)
	if err != nil {
		t.Fatal(err)
	}
	defer device.Close()
	if err := linux.MountExt4(device.Path(), target); err != nil {
		t.Fatal(err)
	}
	return image, target
}

func TestANewRootIsReadableUnderARestrictiveUmaskAndAUsersModeStays(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	base, root := filepath.Join(directory, "base"), filepath.Join(directory, "root")
	for _, path := range []string{base, root} {
		if err := os.Mkdir(path, 0o755); err != nil {
			t.Fatal(err)
		}
	}
	// The service runs with umask 077.
	previous := umask(0o077)
	defer umask(previous)
	_, volume := mountImage(t, directory, "system")
	if err := linux.MountSystemOverlay(base, volume, root); err != nil {
		t.Fatal(err)
	}
	mode := func(path string) fs.FileMode {
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		return info.Mode().Perm()
	}
	if got := mode(root); got != 0o755 {
		t.Errorf("a new root has mode %o", got)
	}
	if err := os.Chmod(root, 0o500); err != nil {
		t.Fatal(err)
	}
	if err := linux.Unmount(root); err != nil {
		t.Fatal(err)
	}
	if err := linux.MountSystemOverlay(base, volume, root); err != nil {
		t.Fatal(err)
	}
	if got := mode(root); got != 0o500 {
		t.Errorf("the mode a user set is now %o", got)
	}
	for _, path := range []string{root, volume} {
		if err := linux.Unmount(path); err != nil {
			t.Fatal(err)
		}
	}
}

func TestALoopDeviceDetachesOnUnmountAndAfterAFailedMount(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	image, target := mountImage(t, directory, "home")
	if !roottest.LoopAttached(t, image) {
		t.Fatal("no loop device holds the image")
	}
	if root, err := linux.IsMountRoot(target); err != nil || !root {
		t.Fatalf("the mount: %v, %v", root, err)
	}
	if err := linux.Unmount(target); err != nil {
		t.Fatal(err)
	}
	if root, err := linux.IsMountRoot(target); err != nil || root {
		t.Fatalf("after the unmount: %v, %v", root, err)
	}
	if !roottest.LoopDetaches(t, image) {
		t.Error("the loop device stays attached after the unmount")
	}
	// An image that is not ext4 fails to mount and leaves no device.
	garbage := filepath.Join(directory, "garbage.img")
	file, err := os.Create(garbage)
	if err != nil {
		t.Fatal(err)
	}
	if err := file.Truncate(8 << 20); err != nil {
		t.Fatal(err)
	}
	file.Close()
	device, err := linux.AttachLoop(garbage)
	if err != nil {
		t.Fatal(err)
	}
	if err := linux.MountExt4(device.Path(), target); err == nil {
		t.Error("garbage was mounted")
	}
	device.Close()
	if !roottest.LoopDetaches(t, garbage) {
		t.Error("the loop device stays attached after a failed mount")
	}
	if root, err := linux.IsMountRoot(filepath.Join(directory, "absent")); err != nil || root {
		t.Errorf("a path with nothing: %v, %v", root, err)
	}
}

func TestEveryFrozenFilesystemIsThawedOnEveryPath(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	var mounts []string
	for _, name := range []string{"system", "home"} {
		_, target := mountImage(t, directory, name)
		mounts = append(mounts, target)
	}
	// A thaw of a filesystem that is not frozen reports it.
	if thawed, err := linux.Thaw(mounts[0]); err != nil || thawed {
		t.Fatalf("thawing what is not frozen: %v, %v", thawed, err)
	}
	var frozen linux.Frozen
	for _, mount := range mounts {
		if err := frozen.Freeze(mount); err != nil {
			t.Fatal(err)
		}
	}
	if failed := frozen.ThawAll(); len(failed) != 0 {
		t.Fatalf("thawing both: %v", failed)
	}
	if thawed, err := linux.Thaw(mounts[0]); err != nil || thawed {
		t.Fatalf("after the thaw: %v, %v", thawed, err)
	}
	// A panic inside the window still thaws what was frozen.
	func() {
		defer func() {
			if recover() == nil {
				t.Error("the job did not panic")
			}
		}()
		var frozen linux.Frozen
		defer frozen.Release()
		for _, mount := range mounts {
			if err := frozen.Freeze(mount); err != nil {
				t.Fatal(err)
			}
		}
		panic("the copy failed")
	}()
	for _, mount := range mounts {
		if thawed, err := linux.Thaw(mount); err != nil || thawed {
			t.Errorf("%s stayed frozen: %v, %v", mount, thawed, err)
		}
		// A frozen filesystem would block this write.
		if err := os.WriteFile(filepath.Join(mount, "written"), []byte("after"), 0o644); err != nil {
			t.Error(err)
		}
		if err := linux.Unmount(mount); err != nil {
			t.Error(err)
		}
	}
}

func TestAJobInAnotherNamespaceNeverLeavesItsThreadInTheProcess(t *testing.T) {
	roottest.Require(t)
	own, err := os.Readlink("/proc/self/ns/net")
	if err != nil {
		t.Fatal(err)
	}
	var inside string
	err = linux.InNewNetworkNamespace(func() error {
		inside, err = os.Readlink("/proc/thread-self/ns/net")
		return err
	})
	if err != nil || inside == own {
		t.Fatalf("the job's namespace %q, the process's %q, %v", inside, own, err)
	}
	// The process, as /proc names it, and every goroutine that runs later stay in
	// the manager's namespace: the job's thread ended with it.
	if now, err := os.Readlink("/proc/self/ns/net"); err != nil || now != own {
		t.Errorf("the process moved to %q: %v", now, err)
	}
	results := make(chan string)
	for range 64 {
		go func() {
			link, _ := os.Readlink("/proc/thread-self/ns/net")
			results <- link
		}()
	}
	for range 64 {
		if link := <-results; link != own {
			t.Fatalf("a goroutine ran in %q, not the process's %q", link, own)
		}
	}
	// A job that cannot enter fails without running.
	ran := false
	err = linux.InNetworkNamespace("/run/netns/absent", func() error {
		ran = true
		return nil
	})
	if !errors.Is(err, fs.ErrNotExist) || ran {
		t.Errorf("entering an absent namespace: %v, ran %v", err, ran)
	}
}
