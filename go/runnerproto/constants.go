package runnerproto

import "time"

// Protocol version and bounds shared by both ends of the runner connection.
const (
	Version            = 24
	MaxMessageBytes    = 4 * 1024 * 1024
	JobViewBytes       = 32 * 1024
	JobLiveBytes       = 16 * 1024
	JobLiveInterval    = 250 * time.Millisecond
	JobGrowthInterval  = 2 * time.Second
	StdinChunkBytes    = 64 * 1024
	LogReadLines       = 1000
	ServiceStderrChars = 16 * 1024
	JobKeptBytes       = 16 * 1024 * 1024
	JobKeptPartBytes   = JobKeptBytes / 2
	JobKeptReadBytes   = JobKeptBytes + 32
	// ArtifactsPath is where an image embeds command-package executables.
	ArtifactsPath = "/opt/demi/artifacts"
)
