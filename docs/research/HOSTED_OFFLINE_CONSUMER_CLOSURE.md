# Hosted Offline Consumer Closure

This report records the hosted clean-consumer failures and the changes that
move the failure boundary toward a fully managed, offline build.

## Run #14

Root cause: the ScriptC child process resolved clang and ar by executable name.
The consumer test intentionally hid host compilers, so the build stopped at
spawn clang ENOENT.

Fix: commit 56b08e4 made ScriptC prepend the managed toolchain bin directory to
its child process environment and added the managed ar alias.

Result: fixed. The failure boundary moved beyond managed ScriptC compilation.

## Run #15

Root cause: the PolkaVM target selector still called
RustcVersion::Autodetect, which attempted bare rustc --version before Cargo
received the managed RUSTC path.

Managed rustc was already present and verified by doctor --json.

Classification: managed target-selection resolver gap.

## Run #16

Commit: 59d0044

Run: [Build JamScript Toolchain Bundle #16](https://github.com/ArcheLabs/JamScript/actions/runs/33944913352)

Job: pending hosted result.

Target selection: rustc_1_91, sourced from toolchains/polkavm.lock.

Managed Rust verification: implemented with the absolute managed rustc path;
the reported sysroot must contain a Rustup channel manifest whose date matches
the locked nightly channel. This avoids confusing the compiler build date with
the Rustup channel date.

Host Rust visibility: must remain hidden. The consumer PATH is not widened for
Rust, Cargo, LLVM, Node, or ScriptC.

Offline consumer build: pending hosted result.

Release publication remains disabled; this validation run does not promote a
bundle.
