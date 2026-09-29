package storage

import (
	"context"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/google/uuid"
)

var ErrObjectNotFound = errors.New("object not found")
var ErrObjectExists = errors.New("object already exists")

type ObjectMeta struct {
	Key          string
	Size         int64
	LastModified time.Time
	Metadata     map[string]string
}

// ObjectStore owns conditional publication and reads in one object namespace.
type ObjectStore interface {
	Head(context.Context, string) (ObjectMeta, error)
	Get(context.Context, string) ([]byte, error)
	Create(context.Context, string, []byte, map[string]string) error
	List(context.Context, string) ([]ObjectMeta, error)
	Delete(context.Context, string) error
}
type LocalObjects struct {
	root *os.Root
}

func OpenLocalObjects(directory string) (*LocalObjects, error) {
	root, err := os.OpenRoot(directory)
	if err != nil {
		return nil, err
	}
	return &LocalObjects{root}, nil
}
func (s *LocalObjects) Close() error { return s.root.Close() }
func localObjectError(err error) error {
	if errors.Is(err, os.ErrNotExist) {
		return ErrObjectNotFound
	}
	if errors.Is(err, os.ErrExist) {
		return ErrObjectExists
	}
	return err
}
func (s *LocalObjects) Head(ctx context.Context, key string) (ObjectMeta, error) {
	if err := ctx.Err(); err != nil {
		return ObjectMeta{}, err
	}
	info, err := s.root.Stat(filepath.FromSlash(key))
	if err != nil {
		return ObjectMeta{}, localObjectError(err)
	}
	if !info.Mode().IsRegular() {
		return ObjectMeta{}, ErrObjectNotFound
	}
	return ObjectMeta{Key: key, Size: info.Size(), LastModified: info.ModTime()}, nil
}
func (s *LocalObjects) Get(ctx context.Context, key string) ([]byte, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	bytes, err := s.root.ReadFile(filepath.FromSlash(key))
	return bytes, localObjectError(err)
}
func (s *LocalObjects) Create(ctx context.Context, key string, data []byte, metadata map[string]string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	name := filepath.FromSlash(key)
	parent := filepath.Dir(name)
	if err := s.root.MkdirAll(parent, 0755); err != nil {
		return err
	}
	// Publish a complete staged object with a hard link: readers cannot observe
	// a partial write, and a concurrent winner is never replaced.
	staged := filepath.Join(parent, ".demi-object-"+uuid.NewString())
	temp, err := s.root.OpenFile(staged, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return err
	}
	defer s.root.Remove(staged)
	_, writeErr := temp.Write(data)
	closeErr := temp.Close()
	if err = errors.Join(writeErr, closeErr); err != nil {
		return err
	}
	if err = ctx.Err(); err != nil {
		return err
	}
	return localObjectError(s.root.Link(staged, name))

}
func (s *LocalObjects) List(ctx context.Context, prefix string) ([]ObjectMeta, error) {
	entries := []ObjectMeta{}
	start := strings.TrimSuffix(prefix, "/")
	if prefix == "" || !strings.HasSuffix(prefix, "/") {
		start = filepath.ToSlash(filepath.Dir(prefix))
	}
	err := fs.WalkDir(s.root.FS(), start, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if err = ctx.Err(); err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		if !strings.HasPrefix(path, prefix) || strings.HasPrefix(entry.Name(), ".demi-object-") {
			return nil
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		entries = append(entries, ObjectMeta{Key: path, Size: info.Size(), LastModified: info.ModTime()})
		return nil
	})
	if errors.Is(err, os.ErrNotExist) {
		err = nil
	}
	return entries, err
}
func (s *LocalObjects) Delete(ctx context.Context, key string) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	return localObjectError(s.root.Remove(filepath.FromSlash(key)))
}
func objectError(err error) error {
	if err == nil {
		return nil
	}
	return fmt.Errorf("the object store failed: %w", err)
}
