# Native transport measurements

Measured September 13, 2026 with Rust 1.98.1 release builds of
`packages/command-service/examples/benchmark.rs`. The fixture launches one
resident native service and reuses its HTTP/2 connection over process pipes.
These are synthetic transport measurements, not file-command or model timings.

macOS arm64 ran on the development machine. Linux arm64 ran as native arm64
code inside the local Lima/KVM host; its executable was statically linked musl.
Builds and other validation ran concurrently, so values represent an observed
run rather than a controlled performance comparison.

| Measurement | macOS arm64 | Linux arm64 |
| --- | ---: | ---: |
| Startup (µs) | 2,361 | 4,646 |
| Warm p50 (µs) | 193 | 555 |
| Warm p95 (µs) | 326 | 683 |
| Concurrent calls/s | 31,381.2 | 34,622.9 |
| Binary MiB/s | 289.2 | 90.7 |
| Slow consumer, 8 MiB (ms) | 511 | 566 |
| Call beside blocked output (µs) | 1,157 | 1,105 |
| Cancel through process reap (µs) | 1,480 | 668 |
| Peak combined RSS (bytes) | Not measured | 4,415,488 |

The warm sample contains 1,000 sequential 32-byte echoes after 20 warmups.
Throughput uses 16 concurrent clients with 100 calls each on one service
connection. Binary throughput transfers and verifies an 8 MiB payload without
base64. The slow reader sleeps 2 ms per output record. Another case leaves an
unbounded producer unread, then executes an independent echo before cancelling
the producer and shutting down/reaping the service. All phases have one overall
60-second bound and explicit cleanup on failure.

Linux RSS samples sum the parent and resident service `/proc/<pid>/status`
values every 10 ms. They exclude compiler, kernel buffers and the VM overhead;
the 4.4 MB observed peak is not a runner or full-application memory measurement.
macOS RSS is not collected by this benchmark. Shorter spikes between samples
can be missed.

The benchmark exposed connection-level flow-control starvation. Both client and
server now advertise a bounded aggregate window for the admitted stream count,
while each stream retains its own 64 KiB window. A regression test in
`command-service/tests/service.rs` verifies that an unread producer does not
block an independent call and that reset reaches the blocked handler.

Run the commands in [Native builds](native-builds.md) to reproduce. Measurements
on other targets and actual file operations remain separate performance work.
