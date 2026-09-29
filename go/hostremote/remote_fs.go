package hostremote

import (
	"context"
	"github.com/wspl/demi/go/core"
	"github.com/wspl/demi/go/runnerproto"
	"github.com/wspl/demi/go/shell"
)

func (h *RemoteHost) Exists(ctx context.Context, path string) (bool, error) {
	value, err := h.fs(ctx, "exists", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSExists{ID: id, Path: path, Cwd: &h.cwd}
	})
	if err != nil {
		return false, err
	}
	result, ok := value.(runnerproto.FSResultExists)
	if !ok {
		return false, protocolError("the runner answered another operation")
	}
	return result.Value, nil
}
func (h *RemoteHost) Stat(ctx context.Context, path string) (shell.FileStat, error) {
	value, err := h.fs(ctx, "stat", func(id string) runnerproto.Inbound { return runnerproto.InboundFSStat{ID: id, Path: path, Cwd: &h.cwd} })
	if err != nil {
		return shell.FileStat{}, err
	}
	result, ok := value.(runnerproto.FSResultStat)
	if !ok {
		return shell.FileStat{}, protocolError("the runner answered another operation")
	}
	return fileStat(result.Value)
}
func (h *RemoteHost) Lstat(ctx context.Context, path string) (shell.FileStat, error) {
	value, err := h.fs(ctx, "lstat", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSLstat{ID: id, Path: path, Cwd: &h.cwd}
	})
	if err != nil {
		return shell.FileStat{}, err
	}
	result, ok := value.(runnerproto.FSResultLstat)
	if !ok {
		return shell.FileStat{}, protocolError("the runner answered another operation")
	}
	return fileStat(result.Value)
}
func (h *RemoteHost) ReadDir(ctx context.Context, path string) ([]shell.DirEntry, error) {
	value, err := h.fs(ctx, "readdir", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSReaddir{ID: id, Path: path, Cwd: &h.cwd}
	})
	if err != nil {
		return nil, err
	}
	result, ok := value.(runnerproto.FSResultReaddir)
	if !ok {
		return nil, protocolError("the runner answered another operation")
	}
	entries := make([]shell.DirEntry, 0, len(result.Value))
	for _, e := range result.Value {
		entries = append(entries, shell.DirEntry{Name: e.Name, Kind: fileKind(runnerproto.FileStat{IsFile: e.IsFile, IsDirectory: e.IsDirectory, IsSymbolicLink: e.IsSymbolicLink})})
	}
	return entries, nil
}
func (h *RemoteHost) Readlink(ctx context.Context, path string) (string, error) {
	value, err := h.fs(ctx, "readlink", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSReadlink{ID: id, Path: path, Cwd: &h.cwd}
	})
	if err != nil {
		return "", err
	}
	result, ok := value.(runnerproto.FSResultReadlink)
	if !ok {
		return "", protocolError("the runner answered another operation")
	}
	return result.Value, nil
}
func (h *RemoteHost) Realpath(ctx context.Context, path string) (string, error) {
	value, err := h.fs(ctx, "realpath", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSRealpath{ID: id, Path: path, Cwd: &h.cwd}
	})
	if err != nil {
		return "", err
	}
	result, ok := value.(runnerproto.FSResultRealpath)
	if !ok {
		return "", protocolError("the runner answered another operation")
	}
	return result.Value, nil
}
func (h *RemoteHost) Mkdir(ctx context.Context, path string, options shell.MkdirOptions) error {
	_, err := h.fs(ctx, "mkdir", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSMkdir{ID: id, Path: path, Cwd: &h.cwd, Recursive: truePointer(options.Recursive)}
	})
	return err
}
func (h *RemoteHost) Rm(ctx context.Context, path string, options shell.RmOptions) error {
	_, err := h.fs(ctx, "rm", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSRm{ID: id, Path: path, Cwd: &h.cwd, Recursive: truePointer(options.Recursive), Force: truePointer(options.Force)}
	})
	return err
}
func (h *RemoteHost) Cp(ctx context.Context, path string, destination string, options shell.CpOptions) error {
	_, err := h.fs(ctx, "cp", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSCp{ID: id, Path: path, Cwd: &h.cwd, Destination: destination, Recursive: truePointer(options.Recursive)}
	})
	return err
}
func (h *RemoteHost) Mv(ctx context.Context, path string, destination string) error {
	_, err := h.fs(ctx, "mv", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSMv{ID: id, Path: path, Cwd: &h.cwd, Destination: destination}
	})
	return err
}
func (h *RemoteHost) Chmod(ctx context.Context, path string, mode uint32) error {
	_, err := h.fs(ctx, "chmod", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSChmod{ID: id, Path: path, Cwd: &h.cwd, Mode: mode}
	})
	return err
}
func (h *RemoteHost) Utimes(ctx context.Context, path string, accessed, modified core.Timestamp) error {
	_, err := h.fs(ctx, "utimes", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSUtimes{ID: id, Path: path, Cwd: &h.cwd, Atime: accessed.Millisecond(), Mtime: modified.Millisecond()}
	})
	return err
}
func (h *RemoteHost) Symlink(ctx context.Context, target, path string) error {
	_, err := h.fs(ctx, "symlink", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSSymlink{ID: id, Path: path, Cwd: &h.cwd, Target: target}
	})
	return err
}
func (h *RemoteHost) Link(ctx context.Context, existing, path string) error {
	_, err := h.fs(ctx, "link", func(id string) runnerproto.Inbound {
		return runnerproto.InboundFSLink{ID: id, Path: path, Cwd: &h.cwd, ExistingPath: existing}
	})
	return err
}
