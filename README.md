# Kestrel

A PS5 → Windows 11 Remote Play client with its own interface, built on
[chiaki-ng](https://github.com/streetpea/chiaki-ng)'s open-source protocol library.

> **Status: Stage 1 of 5 — protocol engine + hardware probe.**
> The desktop UI does not exist yet. Stage 1 delivers the engine and a
> command-line probe that pairs with and streams from a real PS5, so the
> streaming path is proven on hardware before any UI is built on it.
> See [docs/VALIDATION.md](docs/VALIDATION.md) for exactly what has and hasn't been tested.

## Layout

```
native/                 kbridge: small stable C API over libchiaki, built as one DLL
  kbridge.h / .c
  CMakeLists.txt        builds libchiaki (from the submodule) + kbridge
  vcpkg.json            Windows deps: OpenSSL, Opus, json-c, libevent, miniupnpc, protobuf
crates/kestrel-core/    Rust engine: discovery, pairing, sessions, stats, secure storage
crates/kestrel-probe/   Stage-1 CLI: discover / pair / wake / stream (audio, input, recording)
vendor/chiaki-ng/       git submodule, pinned to 43728af (Nov 2026)
docs/                   architecture, Stage 1 test guide, validation matrix
.github/workflows/      Linux tests + Windows x64 build producing kestrel-probe-win64.zip
```

## Getting a Windows build

Push to GitHub; the `build` workflow produces a **kestrel-probe-win64**
artifact (exe + DLLs). Then follow [docs/STAGE1-TESTING.md](docs/STAGE1-TESTING.md).

### Building locally on Windows (optional)

Needs Visual Studio 2022 Build Tools (C++ + Clang), Rust, Python 3 with
`pip install protobuf grpcio-tools`, Ninja, and vcpkg.

```powershell
git clone --recursive <this repo>; cd kestrel
cd vendor\chiaki-ng; git apply --ignore-whitespace --directory=third-party/gf-complete/ scripts/windows-vc/gf-complete.patch; cd ..\..
$env:CC="clang-cl"; $env:CXX="clang-cl"
cmake -S native -B build-native -G Ninja -DCMAKE_BUILD_TYPE=Release `
  -DCMAKE_TOOLCHAIN_FILE="$env:VCPKG_ROOT\scripts\buildsystems\vcpkg.cmake" -DVCPKG_TARGET_TRIPLET=x64-windows
cmake --build build-native --target kbridge
$env:KBRIDGE_LIB_DIR="$PWD\build-native"
cargo build --release -p kestrel-probe
```
Run it from a folder containing `kbridge.dll` and the vcpkg DLLs (`build-native\vcpkg_installed\x64-windows\bin`).

### Linux (development)

```sh
sudo apt install cmake ninja-build protobuf-compiler python3-protobuf libssl-dev libopus-dev \
  libjson-c-dev libevent-dev libminiupnpc-dev libasound2-dev libudev-dev libx11-dev libxext-dev
cmake -S native -B build-native -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build-native --target kbridge
KBRIDGE_LIB_DIR=$PWD/build-native cargo test --workspace
```

## License

AGPL-3.0-only (with chiaki-ng's OpenSSL linking exception), because Kestrel links
chiaki-ng's libchiaki. If you distribute Kestrel or a modified version, its source
must stay available under the same license. Not affiliated with or endorsed by Sony
Interactive Entertainment. Kestrel uses Sony's normal Remote Play pairing; it does
not bypass any console security.
