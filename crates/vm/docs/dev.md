# Quick Start

This documentation is _NOT_ intended to be comprehensive; it is meant to be a quick guide for the most useful things.

### Dependencies

#### macOS and OS X

- `git`
- `Rust 1.85+`
- `LLVM 18`

You'll need LLVM installed and `llvm-config` in your `PATH`. Just download `llvm@18` using `brew`.

```shell
brew install llvm@18 # homebrew installation will automatically install the latest version of llvm
echo 'export PATH="/opt/homebrew/opt/llvm/bin:$PATH"' >> ~/.zshrc # default llvm installtion path using homebrew
source ~/.zshrc
```

Setup a environment variable called `LLVM_SYS_180_PREFIX` pointing to the llvm directory:

```bash
export LLVM_SYS_180_PREFIX="$(brew --prefix llvm@18)"
```

#### Linux

- `git`
- `Rust 1.85+`
- `LLVM 18`

If you are on Debian/Ubuntu, check out the repository [https://apt.llvm.org/](https://apt.llvm.org/) Then you can install with:

```bash
sudo apt-get install llvm-18 llvm-18-dev llvm-18-runtime clang-18 clang-tools-18 lld-18 libpolly-18-dev
```

If you want to build from source, here are some indications:

<details><summary>Install LLVM from source instructions</summary>

```bash
wget https://github.com/llvm/llvm-project/releases/download/llvmorg-18.1.8/llvm-project-18.1.8.src.tar.xz
tar xf llvm-project-18.1.8.src.tar.xz

cd llvm-project-18.1.8.src
mkdir build
cd build

# The following cmake command configures the build to be installed to /opt/llvm-18
cmake -G "Unix Makefiles" ../llvm \
   -DLLVM_ENABLE_PROJECTS="llvm" \
   -DLLVM_BUILD_EXAMPLES=OFF \
   -DLLVM_TARGETS_TO_BUILD="Native" \
   -DCMAKE_INSTALL_PREFIX=/opt/llvm-18 \
   -DCMAKE_BUILD_TYPE=RelWithDebInfo \
   -DLLVM_PARALLEL_LINK_JOBS=4 \
   -DLLVM_ENABLE_BINDINGS=OFF \
   -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DLLVM_ENABLE_LLD=ON \
   -DLLVM_ENABLE_ASSERTIONS=OFF

make -j4
```

</details>

Setup a environment variable called `LLVM_SYS_180_PREFIX` pointing to the llvm directory:

```bash
export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
export PATH=$PATH:/usr/lib/llvm-18/bin
```

Install other dependencies needed by this project.

```shell
apt-get install gcc g++ make git zlib1g-dev zstd libzstd-dev -y
```

### Building

In the top level of the repo and run

```shell
cargo build -r
```

### Testing

#### Unit Testing

In the top level of the repo and run

```shell
cargo test -r
```

#### Snapshot

Install dependencies

```shell
cargo install cargo-insta
```

Run unit tests

```shell
cargo insta test
```

Update snapshots

```shell
cargo insta accept
```

### Formatting

In the top level of the repo and run

```shell
cargo fmt
```

### Linting

In the top level of the repo and run

```shell
cargo clippy --examples --tests -- -D warnings
```
