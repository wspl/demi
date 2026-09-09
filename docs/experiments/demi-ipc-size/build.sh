#!/bin/sh
# macOS ARM64 host; CMAKE and ZIG may point to standalone tool installations.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$root"
out="$root/.cache/demi-ipc-size"
source="$root/docs/experiments/demi-ipc-size"
mkdir -p "$out"
export CMAKE="${CMAKE:-cmake}" ZIG="${ZIG:-zig}"

# Also builds the pinned native libuv archives reused by the C probes.
bun packages/runner/runtime/build.ts "$source/tjs.mjs" "$out/tjs-macos-arm64"
bun packages/runner/runtime/build.ts "$source/tjs.mjs" \
  "$out/tjs-linux-arm64" aarch64-linux-musl

# Persistent object files avoid an Apple linker failure on temporary LTO inputs.
cc -Os -flto=thin -c "$source/posix.c" -o "$out/posix.o"
cc -Os -flto=thin -Wl,-dead_strip "$out/posix.o" -o "$out/posix-macos-arm64"
cc -Os -flto=thin -Ivendor/txiki.js/deps/libuv/include \
  -c "$source/uv.c" -o "$out/uv.o"
cc -Os -flto=thin -Wl,-dead_strip "$out/uv.o" \
  .cache/txiki/host/deps/libuv/libuv.a -o "$out/uv-macos-arm64"
strip "$out/posix-macos-arm64" "$out/uv-macos-arm64"
codesign --force --sign - "$out/posix-macos-arm64"
codesign --force --sign - "$out/uv-macos-arm64"

"$ZIG" cc -target aarch64-linux-musl -Os -flto=thin -s -Wl,--gc-sections \
  "$source/posix.c" -o "$out/posix-linux-arm64"
"$ZIG" cc -target aarch64-linux-musl -Os -flto=thin -s -Wl,--gc-sections \
  -Ivendor/txiki.js/deps/libuv/include "$source/uv.c" \
  .cache/txiki/aarch64-linux-musl/deps/libuv/libuv.a -o "$out/uv-linux-arm64"

cat > "$out/zig-windows-cc" <<'WRAPPER'
#!/bin/sh
exec "$ZIG" cc -target x86_64-windows-gnu "$@"
WRAPPER
cat > "$out/zig-ar" <<'WRAPPER'
#!/bin/sh
exec "$ZIG" ar "$@"
WRAPPER
cat > "$out/zig-ranlib" <<'WRAPPER'
#!/bin/sh
exec "$ZIG" ranlib "$@"
WRAPPER
chmod +x "$out/zig-windows-cc" "$out/zig-ar" "$out/zig-ranlib"
cat > "$out/windows.cmake" <<'TOOLCHAIN'
set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR AMD64)
set(CMAKE_C_COMPILER "${CMAKE_CURRENT_LIST_DIR}/zig-windows-cc")
set(CMAKE_AR "${CMAKE_CURRENT_LIST_DIR}/zig-ar")
set(CMAKE_RANLIB "${CMAKE_CURRENT_LIST_DIR}/zig-ranlib")
set(CMAKE_TRY_COMPILE_TARGET_TYPE STATIC_LIBRARY)
set(CMAKE_C_FLAGS_INIT "-ffunction-sections -fdata-sections -flto=thin")
TOOLCHAIN
"$CMAKE" -S "$source" -B "$out/windows" \
  -DCMAKE_TOOLCHAIN_FILE="$out/windows.cmake" -DCMAKE_BUILD_TYPE=MinSizeRel
"$CMAKE" --build "$out/windows" --target uv_a -j 8
# Direct linking avoids Zig 0.16/CMake's whole-archive Windows CRT link failure.
"$ZIG" cc -target x86_64-windows-gnu -Os -s \
  -Ivendor/txiki.js/deps/libuv/include "$source/uv.c" \
  "$out/windows/libuv/libuv.a" -lpsapi -luser32 -ladvapi32 -liphlpapi \
  -luserenv -lws2_32 -ldbghelp -lole32 -lshell32 -o "$out/uv-windows-x64.exe"
python3 "$source/verify.py" "$out/posix-macos-arm64" \
  "$out/uv-macos-arm64" "$out/tjs-macos-arm64"
