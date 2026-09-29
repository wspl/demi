//go:build linux

package storage_test

import (
	"archive/tar"
	"bytes"
	"context"
	"encoding/binary"
	"errors"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"syscall"
	"testing"

	"github.com/klauspost/compress/zstd"
	"golang.org/x/sys/unix"

	"github.com/wspl/demi/go/machines/internal/imagetest"
	"github.com/wspl/demi/go/machines/internal/linux"
	"github.com/wspl/demi/go/machines/internal/roottest"
	"github.com/wspl/demi/go/machines/internal/storage"
	"github.com/wspl/demi/go/machines/internal/tools"
	"github.com/wspl/demi/go/machinesproto"
)

func TestMain(m *testing.M) {
	os.Exit(roottest.Main(m))
}

// programs resolves the programs on PATH, or skips the test.
func programs(t *testing.T) *tools.Tools {
	t.Helper()
	found, err := tools.Resolve(os.Args[0])
	if err != nil {
		t.Skip(err)
	}
	return found
}

func state(generation string) machinesproto.ImageState {
	return machinesproto.ImageState{
		Generation:  machinesproto.GenerationID(generation),
		BaseVersion: "base",
		SystemBytes: 1024,
		HomeBytes:   1024,
	}
}

func names(t *testing.T, directory string) []string {
	t.Helper()
	entries, err := os.ReadDir(directory)
	if err != nil {
		t.Fatal(err)
	}
	var found []string
	for _, entry := range entries {
		found = append(found, entry.Name())
	}
	return found
}

func inode(t *testing.T, path string) (uint64, uint64) {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	stat := info.Sys().(*syscall.Stat_t)
	return uint64(stat.Dev), stat.Ino
}

func TestAFailedPublicationKeepsTheCommittedPairAndOnlyTwoGenerationsRemain(t *testing.T) {
	root := filepath.Join(t.TempDir(), "images")
	store := storage.NewStore(root)
	device := machinesproto.DeviceID("device")
	source := t.TempDir()
	sources := storage.ImagesIn(source)
	for path, content := range map[string]string{sources.System: "system", sources.Home: "home"} {
		if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if got, err := store.Read(device); got != nil || err != nil {
		t.Fatalf("before the first generation: %v, %v", got, err)
	}
	if err := store.Publish(device, state("first"), sources); err != nil {
		t.Fatal(err)
	}

	if err := os.Remove(sources.Home); err != nil {
		t.Fatal(err)
	}
	if err := store.Publish(device, state("partial"), sources); err == nil {
		t.Fatal("a publication of a missing image succeeded")
	}
	if got, err := store.Read(device); err != nil || got == nil || got.Generation != "first" {
		t.Fatalf("the committed generation after a failure: %v, %v", got, err)
	}
	generations := filepath.Join(root, "device/generations")
	if got := names(t, generations); !slices.Equal(got, []string{"first"}) {
		t.Fatalf("generations %v", got)
	}

	// The generation owns its images: the sources' removal left them.
	committed := store.Images(device, "first")
	if home, err := os.ReadFile(committed.Home); err != nil || string(home) != "home" {
		t.Fatalf("the committed home: %q, %v", home, err)
	}
	for _, volume := range machinesproto.Volumes {
		data, err := os.ReadFile(committed.Get(volume))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(sources.Get(volume), data, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if err := store.Publish(device, state("second"), sources); err != nil {
		t.Fatal(err)
	}
	second := store.Images(device, "second")
	if err := store.Publish(device, state("third"), second); err != nil {
		t.Fatal(err)
	}
	if got := names(t, generations); !slices.Equal(got, []string{"second", "third"}) {
		t.Errorf("generations %v, want the current and the previous", got)
	}
	if got, err := store.Read(device); err != nil || got == nil || got.Generation != "third" {
		t.Errorf("the current generation: %v, %v", got, err)
	}
}

func TestPublicationLinksItsSources(t *testing.T) {
	store := storage.NewStore(filepath.Join(t.TempDir(), "images"))
	sources := storage.ImagesIn(t.TempDir())
	for _, path := range []string{sources.System, sources.Home} {
		if err := os.WriteFile(path, []byte("image"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if err := store.Publish("device", state("first"), sources); err != nil {
		t.Fatal(err)
	}
	committed := store.Images("device", "first")
	for _, volume := range machinesproto.Volumes {
		device, file := inode(t, sources.Get(volume))
		otherDevice, other := inode(t, committed.Get(volume))
		if device != otherDevice || file != other {
			t.Errorf("%s is a copy, not a link", volume)
		}
	}
}

func TestACorruptRecordIsAnError(t *testing.T) {
	root := t.TempDir()
	store := storage.NewStore(root)
	if err := os.Mkdir(filepath.Join(root, "device"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "device/current.json"), []byte(`{"generation":"g"}`), 0o644); err != nil {
		t.Fatal(err)
	}
	_, err := store.Read("device")
	var corrupt *storage.CorruptError
	if !errors.As(err, &corrupt) {
		t.Errorf("%v", err)
	}
}

func TestACapacityIsBlocksTimesBlockSize(t *testing.T) {
	superblock := func(low, high uint32, log, incompat uint32) []byte {
		block := make([]byte, 1024)
		binary.LittleEndian.PutUint32(block[0x04:], low)
		binary.LittleEndian.PutUint32(block[0x18:], log)
		binary.LittleEndian.PutUint16(block[0x38:], 0xEF53)
		binary.LittleEndian.PutUint32(block[0x60:], incompat)
		binary.LittleEndian.PutUint32(block[0x150:], high)
		return block
	}
	capacity := func(block []byte) (uint64, error) {
		image := filepath.Join(t.TempDir(), "image")
		data := append(make([]byte, 1024), block...)
		if err := os.WriteFile(image, data, 0o644); err != nil {
			t.Fatal(err)
		}
		return storage.Capacity(image)
	}
	if got, err := capacity(superblock(262144, 0, 2, 0)); err != nil || got != 1<<30 {
		t.Errorf("a plain superblock: %d, %v", got, err)
	}
	// The high word counts only with the 64-bit feature.
	if _, err := capacity(superblock(0, 1, 2, 0)); err == nil {
		t.Error("no blocks is no filesystem")
	}
	if got, err := capacity(superblock(0, 1, 2, 0x80)); err != nil || got != (1<<32)*4096 {
		t.Errorf("a 64-bit superblock: %d, %v", got, err)
	}
	notExt4 := superblock(1, 0, 2, 0)
	notExt4[0x38] = 0
	var refused *storage.NotExt4Error
	if _, err := capacity(notExt4); !errors.As(err, &refused) {
		t.Errorf("a superblock without the magic: %v", err)
	}
}

func TestTheSuperblockCapacityMatchesDumpe2fs(t *testing.T) {
	t.Parallel()
	found := programs(t)
	dumpe2fs, err := exec.LookPath("dumpe2fs")
	if err != nil {
		t.Skip(err)
	}
	image := filepath.Join(t.TempDir(), "system.ext4")
	if err := storage.MakeSystem(context.Background(), found, image, 48<<20); err != nil {
		t.Fatal(err)
	}
	printed, err := exec.Command(dumpe2fs, "-h", image).Output()
	if err != nil {
		t.Fatal(err)
	}
	field := func(name string) uint64 {
		for _, line := range strings.Split(string(printed), "\n") {
			if value, ok := strings.CutPrefix(line, name); ok {
				var number uint64
				for _, digit := range strings.TrimSpace(value) {
					number = number*10 + uint64(digit-'0')
				}
				return number
			}
		}
		t.Fatalf("dumpe2fs printed no %s", name)
		return 0
	}
	expected := field("Block count:") * field("Block size:")
	got, err := storage.Capacity(image)
	if err != nil || got != expected || expected != 48<<20 {
		t.Errorf("capacity %d, %v; dumpe2fs %d", got, err, expected)
	}
}

func TestRecoveryCompletesAnInterruptedGrowth(t *testing.T) {
	t.Parallel()
	found := programs(t)
	debugfs, err := exec.LookPath("debugfs")
	if err != nil {
		t.Skip(err)
	}
	directory := t.TempDir()
	root := filepath.Join(directory, "mkhome")
	if err := os.MkdirAll(filepath.Join(root, "demi/work"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(root, "demi/work/a.txt"), []byte("alpha\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	image := filepath.Join(directory, "home.ext4")
	const nominal = 64 << 20
	if err := storage.MakeHome(context.Background(), found, root, image, nominal); err != nil {
		t.Fatal(err)
	}
	if info, err := os.Stat(image); err != nil || info.Size() != nominal {
		t.Fatalf("the image: %v, %v", info, err)
	}
	listed, err := exec.Command(debugfs, "-R", "cat /demi/work/a.txt", image).Output()
	if err != nil || string(listed) != "alpha\n" {
		t.Fatalf("the populated home: %q, %v", listed, err)
	}
	// The file was extended, and the growth stopped before the filesystem grew.
	if err := os.Truncate(image, nominal*2); err != nil {
		t.Fatal(err)
	}
	if got, err := storage.Recover(context.Background(), found, image); err != nil || got != nominal*2 {
		t.Fatalf("recovered capacity %d, %v", got, err)
	}
	// The filesystem now fills its file and is consistent.
	if err := exec.Command("e2fsck", "-fn", image).Run(); err != nil {
		t.Errorf("e2fsck: %v", err)
	}
}

func TestASparseCopyKeepsHolesAndTheDataAtBothEnds(t *testing.T) {
	directory := t.TempDir()
	source := filepath.Join(directory, "source")
	destination := filepath.Join(directory, "copy")
	const size = 64 << 20
	file, err := os.Create(source)
	if err != nil {
		t.Fatal(err)
	}
	// A written block of zeros is copied as a hole too.
	for offset, data := range map[int64][]byte{0: []byte("head"), 8 << 20: make([]byte, 1<<20), size - 4: []byte("tail")} {
		if _, err := file.WriteAt(data, offset); err != nil {
			t.Fatal(err)
		}
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	if err := storage.CloneSparse(source, destination); err != nil {
		t.Fatal(err)
	}
	info, err := os.Stat(destination)
	if err != nil {
		t.Fatal(err)
	}
	if info.Size() != size {
		t.Errorf("length %d", info.Size())
	}
	if blocks := info.Sys().(*syscall.Stat_t).Blocks * 512; blocks >= 1<<20 {
		t.Errorf("%d bytes allocated", blocks)
	}
	copied, err := os.Open(destination)
	if err != nil {
		t.Fatal(err)
	}
	defer copied.Close()
	for offset, want := range map[int64]string{0: "head", size - 4: "tail"} {
		got := make([]byte, 4)
		if _, err := copied.ReadAt(got, offset); err != nil || string(got) != want {
			t.Errorf("at %d: %q, %v", offset, got, err)
		}
	}
	if err := storage.CloneSparse(source, destination); err == nil {
		t.Error("the copy is new")
	}
}

func TestLinksAreCopiedAsTheyAreAndEverythingBelongsToTheUser(t *testing.T) {
	directory := t.TempDir()
	skeleton := filepath.Join(directory, "skel")
	if err := os.MkdirAll(filepath.Join(skeleton, ".config"), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(skeleton, ".profile"), []byte("PATH=$HOME/.local/bin:$PATH\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	for name, target := range map[string]string{".bash_profile": ".profile", ".bashrc": "/etc/bash.bashrc"} {
		if err := os.Symlink(target, filepath.Join(skeleton, name)); err != nil {
			t.Fatal(err)
		}
	}
	home := filepath.Join(directory, "demi")
	copied := storage.CopySkeleton(skeleton, home)
	// Giving the entries to the user needs root.
	if os.Geteuid() != 0 {
		if !errors.Is(copied, fs.ErrPermission) {
			t.Fatalf("a copy by an ordinary user: %v", copied)
		}
		return
	}
	if copied != nil {
		t.Fatal(copied)
	}
	for name, want := range map[string]string{".bash_profile": ".profile", ".bashrc": "/etc/bash.bashrc"} {
		if got, err := os.Readlink(filepath.Join(home, name)); err != nil || got != want {
			t.Errorf("%s -> %q, %v", name, got, err)
		}
	}
	for _, path := range []string{home, filepath.Join(home, ".config"), filepath.Join(home, ".profile"), filepath.Join(home, ".bashrc")} {
		info, err := os.Lstat(path)
		if err != nil {
			t.Fatal(err)
		}
		stat := info.Sys().(*syscall.Stat_t)
		if stat.Uid != 1000 || stat.Gid != 1000 {
			t.Errorf("%s belongs to %d:%d", path, stat.Uid, stat.Gid)
		}
	}
}

func TestANewHomeImageHoldsTheUsersDirectoryUnderAReadableRoot(t *testing.T) {
	roottest.Require(t)
	found := programs(t)
	debugfs, err := exec.LookPath("debugfs")
	if err != nil {
		t.Skip(err)
	}
	directory := t.TempDir()
	skeleton := filepath.Join(directory, "skel")
	if err := os.Mkdir(skeleton, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(skeleton, ".profile"), []byte("export EDITOR=vi\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	root := filepath.Join(directory, "mkhome")
	if err := os.Mkdir(root, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.Chmod(root, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := storage.CopySkeleton(skeleton, filepath.Join(root, "demi")); err != nil {
		t.Fatal(err)
	}
	image := filepath.Join(directory, "home.ext4")
	if err := storage.MakeHome(context.Background(), found, root, image, 32<<20); err != nil {
		t.Fatal(err)
	}
	stat := func(path string) string {
		out, err := exec.Command(debugfs, "-R", "stat "+path, image).Output()
		if err != nil {
			t.Fatal(err)
		}
		return string(out)
	}
	if top := stat("/"); !strings.Contains(top, "Mode:  0755") {
		t.Errorf("the root of the home image:\n%s", top)
	}
	if home := stat("/demi"); !strings.Contains(home, "User:  1000") || !strings.Contains(home, "Group:  1000") {
		t.Errorf("the user's directory:\n%s", home)
	}
	if profile := stat("/demi/.profile"); !strings.Contains(profile, "User:  1000") {
		t.Errorf("the profile:\n%s", profile)
	}
}

// vetArchive writes an archive of entries, whose names are written into the
// header directly, so unsafe ones can be made, and vets it.
func vetArchive(t *testing.T, entries []tar.Header) error {
	t.Helper()
	path := filepath.Join(t.TempDir(), "rootfs.tar.zst")
	file, err := os.Create(path)
	if err != nil {
		t.Fatal(err)
	}
	encoder, err := zstd.NewWriter(file)
	if err != nil {
		t.Fatal(err)
	}
	writer := tar.NewWriter(encoder)
	for _, header := range entries {
		if header.Typeflag != tar.TypeXGlobalHeader {
			header.Mode = 0o644
		}
		if err := writer.WriteHeader(&header); err != nil {
			t.Fatal(err)
		}
	}
	for _, closer := range []io.Closer{writer, encoder, file} {
		if err := closer.Close(); err != nil {
			t.Fatal(err)
		}
	}
	return storage.VetArchive(path)
}

func TestSafeEntriesPassAndUnsafeOnesAreRefused(t *testing.T) {
	if err := vetArchive(t, []tar.Header{
		{Typeflag: tar.TypeDir, Name: "./usr/"},
		{Typeflag: tar.TypeReg, Name: "./usr/bin/tini"},
		{Typeflag: tar.TypeSymlink, Name: "./bin", Linkname: "/usr/bin"},
		{Typeflag: tar.TypeLink, Name: "./usr/bin/init", Linkname: "./usr/bin/tini"},
		{Typeflag: tar.TypeXGlobalHeader, PAXRecords: map[string]string{"comment": "x"}},
	}); err != nil {
		t.Errorf("safe entries: %v", err)
	}
	for name, test := range map[string]struct {
		entry tar.Header
		want  error
	}{
		"an absolute path":       {tar.Header{Typeflag: tar.TypeReg, Name: "/etc/passwd"}, storage.ErrUnsafeEntry},
		"a path that climbs":     {tar.Header{Typeflag: tar.TypeReg, Name: "./../escape"}, storage.ErrUnsafeEntry},
		"a fifo":                 {tar.Header{Typeflag: tar.TypeFifo, Name: "./run/pipe"}, storage.ErrUnsafeEntry},
		"a character device":     {tar.Header{Typeflag: tar.TypeChar, Name: "./dev/null"}, storage.ErrUnsafeEntry},
		"a block device":         {tar.Header{Typeflag: tar.TypeBlock, Name: "./dev/sda"}, storage.ErrUnsafeEntry},
		"a hardlink that climbs": {tar.Header{Typeflag: tar.TypeLink, Name: "./etc/shadow", Linkname: "../../etc/shadow"}, storage.ErrUnsafeHardlink},
		"an absolute hardlink":   {tar.Header{Typeflag: tar.TypeLink, Name: "./etc/shadow", Linkname: "/etc/shadow"}, storage.ErrUnsafeHardlink},
	} {
		if err := vetArchive(t, []tar.Header{test.entry}); !errors.Is(err, test.want) {
			t.Errorf("%s: %v, want %v", name, err, test.want)
		}
	}
}

func hostArchitecture(t *testing.T) machinesproto.Architecture {
	t.Helper()
	architecture, ok := machinesproto.HostArchitecture()
	if !ok {
		t.Skip("no Cloud images for this architecture")
	}
	return architecture
}

func otherArchitecture(host machinesproto.Architecture) machinesproto.Architecture {
	if host == machinesproto.Arm64 {
		return machinesproto.Amd64
	}
	return machinesproto.Arm64
}

func TestAVerifiedBaseIsImportedOnceUnderItsManifestDigest(t *testing.T) {
	found := programs(t)
	image := imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, imagetest.Tini, {Path: "/usr/sbin/init", Body: []byte("tini")}}, hostArchitecture(t))
	bases := t.TempDir()
	ctx := context.Background()
	version, err := storage.ImportBase(ctx, found, image.Dir, bases)
	if err != nil {
		t.Fatal(err)
	}
	manifest, err := os.ReadFile(filepath.Join(image.Dir, "manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	if string(version) != imagetest.Digest(manifest) {
		t.Errorf("version %s", version)
	}
	base := filepath.Join(bases, string(version))
	if saved, err := os.ReadFile(filepath.Join(base, "manifest.json")); err != nil || !bytes.Equal(saved, manifest) {
		t.Errorf("the saved manifest: %v", err)
	}
	if tini, err := os.ReadFile(filepath.Join(base, "rootfs/usr/bin/tini")); err != nil || string(tini) != "tini" {
		t.Errorf("tini: %q, %v", tini, err)
	}
	if _, err := os.Stat(filepath.Join(base, "rootfs.tar.zst")); !errors.Is(err, fs.ErrNotExist) {
		t.Errorf("the archive stays: %v", err)
	}
	// A second import finds it; a stale stage is removed.
	if err := os.Mkdir(filepath.Join(bases, ".base-stale"), 0o755); err != nil {
		t.Fatal(err)
	}
	if again, err := storage.ImportBase(ctx, found, image.Dir, bases); err != nil || again != version {
		t.Errorf("the second import: %s, %v", again, err)
	}
	if got := names(t, bases); !slices.Equal(got, []string{string(version)}) {
		t.Errorf("bases %v", got)
	}
	// The same version with other stored bytes is refused.
	if err := os.WriteFile(filepath.Join(base, "manifest.json"), []byte("{}"), 0o644); err != nil {
		t.Fatal(err)
	}
	if _, err := storage.ImportBase(ctx, found, image.Dir, bases); !errors.Is(err, storage.ErrDiffers) {
		t.Errorf("a base with other bytes: %v", err)
	}
}

func TestAReleaseThatFailsACheckIsRefusedAndLeavesNoStage(t *testing.T) {
	found := programs(t)
	host := hostArchitecture(t)
	bases := t.TempDir()
	escaping := append(imagetest.Entries(), imagetest.Entry{Path: "usr/bin/escape", Link: "/etc/hostname"})
	badDigest := imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, imagetest.Tini}, host)
	badDigest.Edit(t, func(manifest map[string]any) {
		manifest["rootfs"].(map[string]any)["sha256"] = strings.Repeat("0", 64)
	})
	for name, test := range map[string]struct {
		image *imagetest.Image
		want  string
	}{
		"an archive that is not the described one": {badDigest, "Cloud root archive integrity mismatch"},
		"another architecture":                     {imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, imagetest.Tini}, otherArchitecture(host)), "architecture differs"},
		"no init":                                  {imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner}, host), "lacks /usr/bin/tini"},
		"an executable the archive lacks":          {imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, imagetest.Tini, {Path: "/usr/bin/demi-helper", Body: []byte("helper")}}, host), "no such file or directory"},
		"an executable that is not the described":  {imagetest.New(t, imagetest.Entries(), []imagetest.Executable{imagetest.Runner, {Path: "/usr/bin/tini", Body: []byte("other")}}, host), "integrity mismatch: /usr/bin/tini"},
		"an executable that is a link out":         {imagetest.New(t, escaping, []imagetest.Executable{imagetest.Runner, imagetest.Tini, {Path: "/usr/bin/escape", Body: []byte("host")}}, host), "Invalid image executable path: /usr/bin/escape"},
	} {
		_, err := storage.ImportBase(context.Background(), found, test.image.Dir, bases)
		if err == nil || !strings.Contains(err.Error(), test.want) {
			t.Errorf("%s: %v, want %q", name, err, test.want)
		}
		if left := names(t, bases); len(left) != 0 {
			t.Errorf("%s left %v", name, left)
		}
	}
}

// The design's acceptance of the in-process copy: on each filesystem it produces
// the source's bytes and allocates no more blocks than
// cp --reflink=auto --sparse=always does. The filesystems a host has tools for
// are tried; ext4 is on every host.
func TestACopyMatchesCpOnEachFilesystemTheHostCanMake(t *testing.T) {
	roottest.Require(t)
	directory := t.TempDir()
	// A sparse ext4 image with files, a written run of zeros and data at both ends,
	// like a working image.
	content := filepath.Join(directory, "content")
	if err := os.Mkdir(content, 0o755); err != nil {
		t.Fatal(err)
	}
	for index := range 8 {
		body := bytes.Repeat([]byte{byte(index + 1)}, 300_000+index*4096)
		if err := os.WriteFile(filepath.Join(content, "file-"+string(rune('0'+index))), body, 0o644); err != nil {
			t.Fatal(err)
		}
	}
	source := filepath.Join(directory, "source.ext4")
	if output, err := exec.Command("mke2fs", "-q", "-t", "ext4", "-d", content, source, "96m").CombinedOutput(); err != nil {
		t.Skipf("mke2fs: %v\n%s", err, output)
	}
	file, err := os.OpenFile(source, os.O_WRONLY, 0)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteAt(make([]byte, 2<<20), 40<<20); err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteAt([]byte("tail"), 96<<20-4); err != nil {
		t.Fatal(err)
	}
	file.Close()
	for _, filesystem := range []struct{ name, mkfs, quiet string }{{"xfs", "mkfs.xfs", "-f"}, {"btrfs", "mkfs.btrfs", "-f"}, {"ext4", "mkfs.ext4", "-F"}} {
		t.Run(filesystem.name, func(t *testing.T) {
			mkfs, err := exec.LookPath(filesystem.mkfs)
			if err != nil {
				t.Skip(err)
			}
			image := filepath.Join(directory, filesystem.name+".img")
			if err := os.WriteFile(image, nil, 0o644); err != nil {
				t.Fatal(err)
			}
			if err := os.Truncate(image, 512<<20); err != nil {
				t.Fatal(err)
			}
			device, err := linux.AttachLoop(image)
			if err != nil {
				t.Fatal(err)
			}
			if output, err := exec.Command(mkfs, "-q", filesystem.quiet, device.Path()).CombinedOutput(); err != nil {
				device.Close()
				t.Fatalf("%s: %v\n%s", mkfs, err, output)
			}
			target := filepath.Join(directory, filesystem.name)
			if err := os.Mkdir(target, 0o755); err != nil {
				t.Fatal(err)
			}
			mounted := syscall.Mount(device.Path(), target, filesystem.name, 0, "")
			device.Close()
			if mounted != nil {
				t.Fatal(mounted)
			}
			inside := filepath.Join(target, "source.ext4")
			run := func(program string, args ...string) {
				t.Helper()
				if output, err := exec.Command(program, args...).CombinedOutput(); err != nil {
					t.Fatalf("%s %v: %v\n%s", program, args, err, output)
				}
			}
			run("cp", "--sparse=always", source, inside)
			ours, theirs := filepath.Join(target, "ours.ext4"), filepath.Join(target, "theirs.ext4")
			if err := storage.CloneSparse(inside, ours); err != nil {
				t.Fatal(err)
			}
			run("cp", "--reflink=auto", "--sparse=always", inside, theirs)
			run("sync", "-f", target)
			blocks := func(path string) int64 {
				info, err := os.Stat(path)
				if err != nil {
					t.Fatal(err)
				}
				return info.Sys().(*syscall.Stat_t).Blocks
			}
			sha := func(path string) string {
				data, err := os.ReadFile(path)
				if err != nil {
					t.Fatal(err)
				}
				return imagetest.Digest(data)
			}
			if sha(ours) != sha(inside) {
				t.Error("the copy's content differs")
			}
			if info, err := os.Stat(ours); err != nil || info.Size() != 96<<20 {
				t.Errorf("length %v, %v", info, err)
			}
			if blocks(ours) > blocks(theirs) {
				t.Errorf("%d blocks against cp's %d", blocks(ours), blocks(theirs))
			}
			t.Logf("%s: ours %d blocks, cp %d blocks, source %d", filesystem.name, blocks(ours), blocks(theirs), blocks(inside))
			if err := linux.Unmount(target); err != nil {
				t.Fatal(err)
			}
			// Unmounting detaches the auto-clear loop device.
			if !roottest.LoopDetaches(t, image) {
				t.Error("the loop device stays attached")
			}
		})
	}
}

// A growth the kernel refuses for want of CAP_SYS_RESOURCE names the capability
// (docs/cloud/managed-hosts.md § Lifecycle and capacity). A host that lacks the
// capability, as a container may, is the case itself; on one that has it, the
// test starts itself again with the capability removed from its bounding set,
// which resize2fs inherits.
func TestAGrowthTheKernelRefusesForWantOfCapSysResourceNamesTheCapability(t *testing.T) {
	roottest.Require(t)
	held, err := unix.PrctlRetInt(unix.PR_CAPBSET_READ, unix.CAP_SYS_RESOURCE, 0, 0, 0)
	if err != nil {
		t.Fatal(err)
	}
	if held == 1 {
		setpriv, err := exec.LookPath("setpriv")
		if err != nil {
			t.Skip(err)
		}
		command := exec.Command(setpriv, "--bounding-set=-sys_resource", os.Args[0], "-test.run=^TestAGrowthTheKernelRefusesForWantOfCapSysResourceNamesTheCapability$", "-test.v")
		if output, err := command.CombinedOutput(); err != nil || strings.Contains(string(output), "SKIP") {
			t.Fatalf("without the capability: %v\n%s", err, output)
		}
		return
	}
	found := programs(t)
	directory := t.TempDir()
	image := filepath.Join(directory, "volume.ext4")
	const nominal = 32 << 20
	if err := storage.MakeSystem(context.Background(), found, image, nominal); err != nil {
		t.Fatal(err)
	}
	target := filepath.Join(directory, "volume")
	if err := os.Mkdir(target, 0o755); err != nil {
		t.Fatal(err)
	}
	device, err := linux.AttachLoop(image)
	if err != nil {
		t.Fatal(err)
	}
	if err := linux.MountExt4(device.Path(), target); err != nil {
		device.Close()
		t.Fatal(err)
	}
	number := device.Number()
	// The mount holds the device from here on.
	device.Close()
	if err := os.Truncate(image, nominal*2); err != nil {
		t.Fatal(err)
	}
	if err := linux.RefreshLoopCapacity(number); err != nil {
		t.Fatal(err)
	}
	refused := storage.GrowMounted(context.Background(), found, linux.LoopPath(number))
	if err := linux.Unmount(target); err != nil {
		t.Fatal(err)
	}
	want := "growing a mounted ext4 filesystem needs CAP_SYS_RESOURCE; this host's manager lacks it"
	if refused == nil || refused.Error() != want || !errors.Is(refused, storage.ErrGrowthCapability) {
		t.Errorf("%v, want %q", refused, want)
	}
	if got, err := storage.Capacity(image); err != nil || got != nominal {
		t.Errorf("the filesystem's capacity is %d, %v; want %d", got, err, nominal)
	}
}
