# txiki.js size experiment

Measured on 2026-09-08. This experiment evaluates a smaller build of upstream
[txiki.js v26.6.0](https://github.com/saghul/txiki.js/releases/tag/v26.6.0), commit
`1a230d31183f062fae7a6c4fd2cff466cecc1787`, with the submodules pinned by that commit.
It does not replace Demi's tinyjs runtime or implement a txiki.js runner adapter.

## Result

All sizes below are the runtime executable alone, without Demi's runner bundle.
One MiB is 1,048,576 bytes. Gzip sizes use level 9 and a zero timestamp.

| Platform / build | Executable bytes | MiB | Gzip bytes |
| --- | ---: | ---: | ---: |
| macOS ARM64, official release | 5,913,888 | 5.640 | — |
| macOS ARM64, trimmed | 2,622,016 | 2.501 | 1,244,280 |
| Linux ARM64, full features, optimized, dynamic | 4,117,832 | 3.927 | — |
| Linux ARM64, trimmed, dynamic | 2,774,032 | 2.646 | 1,273,500 |
| Linux ARM64, trimmed, static glibc | 4,163,488 | 3.971 | 1,950,765 |
| Linux ARM64, trimmed, static musl + ThinLTO | **2,853,504** | **2.721** | **1,392,874** |

The Linux full and trimmed dynamic builds use the same compiler and optimization
options: removing the four optional features reduces their executable size by
32.6%. The macOS comparison also includes optimization differences from the
upstream release build, so its 55.7% reduction cannot be attributed to feature
removal alone.

For context, the existing local tinyjs Linux ARM64 musl release executable measured
2,746,320 bytes (2.619 MiB). The trimmed txiki.js musl executable is 107,184 bytes
larger (3.9%). This is a size comparison, not a performance or feature equivalence
claim. Neither measurement includes the runner bundle.

## Features and dependencies

Disabled using upstream CMake options, with no changes to upstream runtime source:

- WebAssembly / WAMR.
- SQLite and the optional SQLite module.
- FFI and its libffi dependency.
- mimalloc; the runtime uses the target's C allocator instead.

Kept: QuickJS-ng, ESM loading, file and process I/O, signals, TCP/Unix sockets,
HTTP client/server, WebSocket, TLS and embedded CA roots, Web Streams, Fetch,
encoding, URL, events/cancellation and Web Crypto. Other upstream features,
including workers, compression and the CLI, remain compiled in. This is a first
trim using existing switches, not the smallest theoretically possible fork.

The dynamic Linux build requires `libstdc++`, `libm`, `libgcc_s`, `libc` and the ELF
loader. Its 2.646 MiB is therefore not a standalone deployment total. The macOS
build depends on the operating system's `libSystem` and `libc++`.

The musl build is a stripped, statically linked AArch64 ELF, with no ELF interpreter
or dynamic-library dependencies. It is the relevant candidate for a standalone
Cloud runtime. The glibc static build is included only as a measurement: its linker
warns that some name/user/group lookup functions can still require matching glibc
shared libraries at runtime.

## Reproduce the builds

Tools used:

- macOS: Apple Clang 21.0.0, CMake 4.4.3.
- Linux VM: Ubuntu 24.04 ARM64, GCC/G++ 13.3.0, CMake 3.28.3, Ninja.
- Linux musl cross-build on macOS: Zig 0.16.0 (Clang 21.1.0), CMake 4.4.3.

Fetch the release and its pinned dependencies:

```sh
git clone --recursive --depth 1 --branch v26.6.0 \
  https://github.com/saghul/txiki.js.git txiki.js
cd txiki.js
```

Native trimmed build, usable on macOS or Linux:

```sh
cmake -S . -B build-trim \
  -DCMAKE_BUILD_TYPE=MinSizeRel \
  -DBUILD_WITH_STRIP=ON \
  -DBUILD_WITH_LTO=ON \
  -DBUILD_WITH_GC_SECTIONS=ON \
  -DBUILD_WITH_WASM=OFF \
  -DBUILD_WITH_SQLITE=OFF \
  -DBUILD_WITH_FFI=OFF \
  -DBUILD_WITH_MIMALLOC=OFF
cmake --build build-trim --target tjs-cli -j 6
```

For the full-feature Linux baseline, use a separate build directory and omit the
four feature-disable options. It additionally needs libffi development files.
For the glibc static measurement, add `-DCMAKE_EXE_LINKER_FLAGS=-static` to the
trimmed native Linux configuration.

For the musl cross-build, set `ZIG` to the absolute path of Zig 0.16.0 and generate
four tool wrappers and a CMake toolchain in the source directory:

```sh
export ZIG=/absolute/path/to/zig
mkdir -p toolchain
printf '#!/bin/sh\nexec "$ZIG" cc -target aarch64-linux-musl "$@"\n' > toolchain/cc
printf '#!/bin/sh\nexec "$ZIG" c++ -target aarch64-linux-musl "$@"\n' > toolchain/cxx
printf '#!/bin/sh\nexec "$ZIG" ar "$@"\n' > toolchain/ar
printf '#!/bin/sh\nexec "$ZIG" ranlib "$@"\n' > toolchain/ranlib
chmod +x toolchain/cc toolchain/cxx toolchain/ar toolchain/ranlib
cat > toolchain/musl.cmake <<EOF_CMAKE
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR aarch64)
set(CMAKE_C_COMPILER "$PWD/toolchain/cc")
set(CMAKE_CXX_COMPILER "$PWD/toolchain/cxx")
set(CMAKE_AR "$PWD/toolchain/ar")
set(CMAKE_RANLIB "$PWD/toolchain/ranlib")
EOF_CMAKE
cmake -S . -B build-musl \
  -DCMAKE_TOOLCHAIN_FILE="$PWD/toolchain/musl.cmake" \
  -DCMAKE_BUILD_TYPE=MinSizeRel \
  '-DCMAKE_C_FLAGS=-s -flto=thin' \
  '-DCMAKE_CXX_FLAGS=-s -flto=thin' \
  '-DCMAKE_EXE_LINKER_FLAGS=-static -flto=thin' \
  -DBUILD_WITH_STRIP=OFF \
  -DBUILD_WITH_LTO=OFF \
  -DBUILD_WITH_GC_SECTIONS=ON \
  -DBUILD_WITH_WASM=OFF \
  -DBUILD_WITH_SQLITE=OFF \
  -DBUILD_WITH_FFI=OFF \
  -DBUILD_WITH_MIMALLOC=OFF
cmake --build build-musl --target tjs-cli -j 6
```

The musl recipe applies ThinLTO explicitly. Upstream's `BUILD_WITH_LTO` detection
failed to locate the compiler-specific archiver for the Zig wrappers; merely
setting that option to ON produced a warning and a non-LTO build (2,922,624 bytes).
The final build uses `-flto=thin` for compilation and linking, and `-s` for link-time
stripping instead of invoking macOS `strip` on an ELF file. Compilation succeeded
with warnings; this was not a warning-free build.

## Verification

Run the selected upstream tests with the included harness on the target platform:

```sh
python3 /path/to/demi/docs/experiments/txiki-size/verify.py \
  /path/to/txiki.js /path/to/tjs
```

The final musl build, macOS trimmed build, and glibc static build each passed 25 of
26 selected upstream tests. The checks cover file streams and metadata, process
execution/termination, encoding, transforms, HTTP streaming and body consumption,
Fetch cancellation, authenticated HTTP proxying, Unix sockets, TLS with custom
CA verification, WebSocket headers/streams and cryptography. No model calls were
made. This is not a full Web Platform Tests run or proof of standards compliance.

The failure is `test-tcp-connect-abort.js`: its in-flight connection check expects
`AbortError` but finds `null` for a connection to `192.0.2.1:81`. The same check fails
on the untrimmed Linux baseline in this environment. This comparison does not
show a trimming regression; the underlying cause remains uninvestigated.
The harness reports the failure and exits nonzero rather than hiding it.

Not verified: the actual Demi runner on txiki.js, PID 1 behavior in Firecracker,
privilege dropping, process-group semantics, command import restrictions, log
fan-out, or cold-start/RSS/throughput performance. Additional native bindings may
be needed before replacing tinyjs, and would change the final executable size.

## Local artifacts

Ignored output is retained in `.cache/txiki-size-2026-09-08/`: the trimmed binaries,
gzip copies, source archive, size/hash manifest and test logs. The original source
checkout is `/tmp/demi-txiki-trim`; the Linux test copy is in the same path inside
the Lima `fc` VM. The VM was stopped again after the experiment.

Final Linux musl executable SHA-256:

```text
8d6da677ece30a5076927e013dc9b157acc93a69d440b2fcb4b55b4e62b61adb
```
