package hostaccess

import (
	"context"
	"errors"
	"fmt"
	"io"
	"slices"
	"strings"
	"sync"

	"github.com/wspl/demi/internal/backend/remotehost"
	"github.com/wspl/demi/internal/gates"
	"github.com/wspl/demi/internal/host"
	"github.com/wspl/demi/internal/plugin"
	"github.com/wspl/demi/internal/webapi"
)

// DirectorySet is one plugin and its desired Host directories. An empty set
// removes the disabled plugin's directories at the next installation.
type DirectorySet struct {
	Plugin      plugin.ID
	Directories []plugin.HostDirectory
}

// DirectorySets is the ordered Host directory sets of the user's plugins.
type DirectorySets []DirectorySet

// PluginInstalls remembers each device's installed revision and connection.
// Its opaque state uses the shard mutex; installation waits occur outside it.
type PluginInstalls struct {
	mu     *sync.Mutex // The shard mutex protects completed connection/revision entries.
	synced map[webapi.DeviceID]installedDirectories
	turn   gates.Serial
}
type installedDirectories struct {
	link     remotehost.WeakLink
	revision string
}

// NewPluginInstalls creates the installation registry for the shard mutex.
func NewPluginInstalls(mu *sync.Mutex) *PluginInstalls {
	return &PluginInstalls{mu: mu, synced: make(map[webapi.DeviceID]installedDirectories)}
}

// ReadFiles reads the main Host as a look, with no activity or wake. A transition
// ends the read. Path failures answer unreadable; answers keep request order.
func ReadFiles(
	ctx context.Context,
	shard HostShard,
	id webapi.ConversationID,
	reads []plugin.HostRead,
) ([]plugin.HostFile, error) {
	access, waitCtx, stop, err := admitStream(ctx, shard, id, false, false)
	if err != nil {
		return nil, readFilesError(err)
	}
	defer stop()
	defer access.release()
	files := make([]plugin.HostFile, 0, len(reads))
	for _, read := range reads {
		file, err := look(waitCtx, access.host.Host.FS(), read)
		if err != nil {
			return nil, readFilesError(accessError(err))
		}
		files = append(files, file)
	}
	return files, nil
}

// readFilesError translates stopped and offline Hosts into the plugin's no-wake refusal.
func readFilesError(err error) error {
	var refusal Refusal
	var failure *host.Error
	if errors.As(err, &refusal) && refusal == Stopped ||
		errors.As(err, &failure) && (failure.Kind == host.Offline || failure.Kind == host.Unavailable) {
		return &ReadFilesError{Kind: ReadFilesNotRunning}
	}
	return &ReadFilesError{Kind: ReadFilesAccess, Cause: err}
}

// look reads one plugin path, distinguishing path failures from transport failure.
func look(ctx context.Context, fs host.FS, read plugin.HostRead) (plugin.HostFile, error) {
	stat, err := fs.Stat(ctx, read.Path)
	if err != nil {
		if hostCode(err, "ENOENT") || hostCode(err, "ENOTDIR") {
			return &plugin.HostFileMissing{}, nil
		}
		return unreadable(err)
	}
	switch stat.Kind {
	case host.Directory:
		entries, err := fs.ReadDir(ctx, read.Path)
		if err != nil {
			return unreadable(err)
		}
		result := make([]plugin.HostEntry, 0, len(entries))
		for _, entry := range entries {
			kind := plugin.EntryKindOther
			switch entry.Kind {
			case host.File:
				kind = plugin.EntryKindFile
			case host.Directory:
				kind = plugin.EntryKindDirectory
			case host.Symlink:
				kind = plugin.EntryKindSymlink
			}
			result = append(result, plugin.HostEntry{Name: entry.Name, Kind: kind})
		}
		return &plugin.HostFileDirectory{Entries: result}, nil
	case host.File:
		stream, err := fs.ReadStream(ctx, read.Path, host.ByteRange{Length: new(read.Limit)})
		if err != nil {
			return unreadable(err)
		}
		defer func() { _ = stream.Close(context.WithoutCancel(ctx)) }() // Remote pipe Close only releases its reader.
		var data []byte
		buffer := make([]byte, 65536)
		for {
			n, err := stream.Read(ctx, buffer)
			data = append(data, buffer[:n]...)
			if errors.Is(err, io.EOF) {
				break
			}
			if err != nil {
				return nil, err
			}
		}
		return &plugin.HostFileFile{Bytes: data, Size: stat.Size}, nil
	default:
		return &plugin.HostFileOther{}, nil
	}
}

// unreadable keeps a path's failure local while a lost Host fails the whole read.
func unreadable(err error) (plugin.HostFile, error) {
	var failure *host.Error
	if errors.As(err, &failure) && failure.Kind == host.Failed {
		return &plugin.HostFileUnreadable{Message: failure.Message}, nil
	}
	return nil, err
}

// installDirectories serializes installation and records only complete revisions.
func installDirectories(
	ctx context.Context,
	shard HostShard,
	device webapi.DeviceID,
	admitted *ConversationHost,
) error {
	link := shard.Devices().Link(device)
	if link == nil {
		return nil
	} // Starting the job reports connection loss.
	sets, err := shard.DirectorySets(ctx)
	if err != nil {
		return &host.Error{Kind: host.Failed, Message: err.Error()}
	}
	var revision strings.Builder
	for _, set := range sets {
		revision.WriteString(string(set.Plugin))
		for _, directory := range set.Directories {
			revision.WriteByte(' ')
			revision.WriteString(directory.HostName())
		}
		revision.WriteByte('\n')
	}
	installs := shard.PluginInstalls()
	wanted := installedDirectories{link: link.Downgrade(), revision: revision.String()}
	installs.mu.Lock()
	previous := installs.synced[device]
	ready := previous.revision == wanted.revision && previous.link.Is(link)
	installs.mu.Unlock()
	if ready {
		return nil
	}
	permit, err := installs.turn.Acquire(ctx)
	if err != nil {
		return err
	}
	defer permit.Release()
	installs.mu.Lock()
	previous = installs.synced[device]
	ready = previous.revision == wanted.revision && previous.link.Is(link)
	installs.mu.Unlock()
	if ready {
		return nil
	}
	if admitted.Home == nil {
		return &host.Error{Kind: host.Failed, Message: "the Host reported no home directory for the plugins' files"}
	}
	if err := syncDirectories(ctx, shard, admitted, sets); err != nil {
		return err
	}

	installs.mu.Lock()
	installs.synced[device] = wanted
	installs.mu.Unlock()
	return nil
}

// installDirectory writes a plugin directory beside its final path before renaming it.
func installDirectory(
	ctx context.Context,
	shard HostShard,
	fs host.FS,
	base string,
	directory plugin.HostDirectory,
) error {
	name := directory.HostName()
	partial := base + "/." + name + ".partial"
	if err := removeDirectory(ctx, fs, partial); err != nil {
		return err
	}
	if err := fs.Mkdir(ctx, partial, host.MkdirOptions{Recursive: true}); err != nil {
		return err
	}
	parents, err := writeDirectoryFiles(ctx, shard, fs, partial, name, directory)
	if err != nil {
		return err
	}
	sorted := make([]string, 0, len(parents))
	for parent := range parents {
		sorted = append(sorted, parent)
	}
	slices.Sort(sorted)
	slices.Reverse(sorted)
	for _, parent := range sorted {
		if err := fs.Chmod(ctx, parent, 0o555); err != nil {
			return err
		}
	}
	installed := base + "/" + name
	if err := fs.Mv(ctx, partial, installed); err != nil {
		return err
	}
	return fs.Chmod(ctx, installed, 0o555)
}

// removeDirectory makes installation-owned directories writable before removing them.
func removeDirectory(ctx context.Context, fs host.FS, path string) error {
	stat, err := fs.Lstat(ctx, path)
	if err != nil {
		if hostCode(err, "ENOENT") || hostCode(err, "ENOTDIR") {
			return nil
		}
		return err
	}
	if stat.Kind == host.Directory {
		pending := []string{path}
		for len(pending) > 0 {
			directory := pending[len(pending)-1]
			pending = pending[:len(pending)-1]
			if err := fs.Chmod(ctx, directory, 0o755); err != nil {
				return err
			}
			entries, err := fs.ReadDir(ctx, directory)
			if err != nil {
				return err
			}
			for _, entry := range entries {
				if entry.Kind == host.Directory {
					pending = append(pending, directory+"/"+entry.Name)
				}
			}
		}
	}
	return fs.Rm(ctx, path, host.RmOptions{Recursive: true, Force: true})
}

func syncDirectories(ctx context.Context, shard HostShard, admitted *ConversationHost, sets DirectorySets) error {
	for _, set := range sets {
		base := *admitted.Home + "/.demi/plugins/" + string(set.Plugin)
		present, err := admitted.Host.FS().ReadDir(ctx, base)
		if err != nil && !hostCode(err, "ENOENT") && !hostCode(err, "ENOTDIR") {
			return err
		}
		names := make(map[string]bool)
		for _, directory := range set.Directories {
			names[directory.HostName()] = true
		}
		exists := make(map[string]bool)
		for _, entry := range present {
			exists[entry.Name] = true
			if !names[entry.Name] {
				if err := removeDirectory(ctx, admitted.Host.FS(), base+"/"+entry.Name); err != nil {
					return err
				}
			}
		}
		for _, directory := range set.Directories {
			if !exists[directory.HostName()] {
				if err := installDirectory(ctx, shard, admitted.Host.FS(), base, directory); err != nil {
					return err
				}
			}
		}
	}
	return nil
}

func writeDirectoryFiles(
	ctx context.Context,
	shard HostShard,
	fs host.FS,
	partial, name string,
	directory plugin.HostDirectory,
) (map[string]bool, error) {
	parents := make(map[string]bool)
	for _, file := range directory.Files {
		data, exists, err := shard.Blobs().Read(ctx, file.Blob)
		if err != nil {
			return nil, &host.Error{Kind: host.Failed, Message: err.Error()}
		}
		if !exists {
			return nil, &host.Error{
				Kind:    host.Failed,
				Message: fmt.Sprintf("the file %s of %s is not stored", file.Path, name),
			}
		}
		path := partial + "/" + file.Path
		if err := fs.WriteFile(
			ctx,
			path,
			host.FileContents{Bytes: data},
			host.WriteOptions{CreateParents: true},
		); err != nil {
			return nil, err
		}
		mode := uint32(0o444)
		if file.Executable {
			mode = 0o555
		}
		if err := fs.Chmod(ctx, path, mode); err != nil {
			return nil, err
		}
		for parent := file.Path; strings.Contains(parent, "/"); {
			parent = parent[:strings.LastIndexByte(parent, '/')]
			parents[partial+"/"+parent] = true
		}
	}
	return parents, nil
}
