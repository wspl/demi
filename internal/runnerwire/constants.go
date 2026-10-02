package runnerwire

import "time"

// The wire's version, which a runner's hello names.
const Version = 24

// The largest frame either end sends.
const MaxMessageBytes = 4 * 1024 * 1024

// How much of the start of each stream a job always sends, and how much of
// its newest bytes beyond them it sends while nobody follows it: the model's
// view of a running command (`runner.md` § Pipes and output).
const JobViewBytes = 8 * 1024

// The most bytes one message beyond a stream's first [`JOB_VIEW_BYTES`]
// carries while the backend follows the job: 4,096 characters of up to four
// bytes each.
const JobLiveBytes = 16 * 1024

// How often a followed job sends each stream's newest bytes beyond its
// first [`JOB_VIEW_BYTES`], at most.
const JobLiveInterval = 250 * time.Millisecond

// How often a job nobody follows sends how long a stream grew beyond its
// first [`JOB_VIEW_BYTES`] and its newest [`JOB_VIEW_BYTES`], at most.
const JobGrowthInterval = 2 * time.Second

// The most bytes of one live stdin frame.
const StdinChunkBytes = 64 * 1024

// The most lines one `log_read` returns.
const LogReadLines = 1000

// The most of an invocation's standard error a `service_done` carries, in
// Unicode scalar values (`contracts.md` § Validation at entry). The runner keeps the
// last this many bytes, which are never more characters.
const ServiceStderrChars = 16 * 1024

// Where an image embeds each command package's executable: alone in a
// directory named by its SHA-256, under the name its release gives it, such
// as `/opt/demi/artifacts/<sha256>/demi-file`.
const ArtifactsPath = "/opt/demi/artifacts"

// The most installs one list carries: one per service starting at once.
const MaxInstalls = 64
