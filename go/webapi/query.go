package webapi

import (
	"github.com/wspl/demi/go/runnerproto"
	"strconv"
)

// StrictBool accepts the two query spellings and defaults to false.
type StrictBool bool

func (v *StrictBool) UnmarshalText(text []byte) error {
	switch string(text) {
	case "true":
		*v = true
	case "false":
		*v = false
	default:
		return &InvalidError{Rule: "must be true or false"}
	}
	return nil
}

type Refresh struct{ Refresh StrictBool }
type ConversationsQuery struct{ Archived StrictBool }
type DirectoryQuery struct{ Path *NonEmptyPath }
type DeviceDirectoryQuery struct{ Path *AbsolutePath }
type FileQuery struct{ Path NonEmptyPath }
type RemoveQuery struct{ Path AbsolutePath }
type RawFileQuery struct {
	Path     NonEmptyPath
	Version  *NonEmptyPath
	Download StrictBool
}

// FileUploadQuery distinguishes files::UploadQuery from attachments::UploadQuery.
type FileUploadQuery struct {
	Path    NonEmptyPath
	Replace StrictBool
}
type TreeFileQuery struct{ Path TreePath }
type CommittedFileQuery struct {
	Path     TreePath
	Download StrictBool
}
type DeviceLogQuery struct {
	Since  *uint64
	Limit  LogLimit
	Source *LogSource
}

// LogLimit's zero value is the omitted query parameter (200 lines).
type LogLimit struct{ value uint64 }

func (v LogLimit) Get() uint64 {
	if v.value == 0 {
		return 200
	}
	return v.value
}
func ParseLogLimit(value uint64) (LogLimit, error) {
	if value < 1 || value > runnerproto.LogReadLines {
		return LogLimit{}, &InvalidError{Rule: "must be 1 to the runner's log read limit"}
	}
	return LogLimit{value: value}, nil
}
func (v *LogLimit) UnmarshalText(text []byte) error {
	value, err := strconv.ParseUint(string(text), 10, 64)
	if err != nil {
		return &InvalidError{Rule: "must be an unsigned integer"}
	}
	next, err := ParseLogLimit(value)
	if err != nil {
		return err
	}
	*v = next
	return nil
}

// LogSource is the exact nonempty source filter of a device log read.
type LogSource struct{ NonEmptyPath }

func ParseLogSource(text string) (LogSource, error) {
	value, err := ParseNonEmptyPath(text)
	return LogSource{NonEmptyPath: value}, err
}
