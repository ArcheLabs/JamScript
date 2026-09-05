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

## Run #16 / #17

Commit: 59d0044

Run: [Build JamScript Toolchain Bundle #16](https://github.com/ArcheLabs/JamScript/actions/runs/33944913352)

Job: failed in the final clean offline consumer smoke step. The earlier bundle
build, verification, reproducibility, CLI preparation, and fixture preparation
steps passed.

Target selection: rustc_1_91, sourced from toolchains/polkavm.lock.

Managed Rust verification: implemented with the absolute managed rustc path;
the reported sysroot must contain a Rustup channel manifest whose date matches
the locked nightly channel. This avoids confusing the compiler build date with
the Rustup channel date.

Host Rust visibility: must remain hidden. The consumer PATH is not widened for
Rust, Cargo, LLVM, Node, or ScriptC.

Offline consumer build: failed; the strict managed Rust channel verification
was refined in commit 26a0084 because the compiler build date can differ from
the Rustup channel date.

Release publication remains disabled; this validation run does not promote a
bundle.

## Run #18

Commit: 26a0084

Run: [Build JamScript Toolchain Bundle #18](https://github.com/ArcheLabs/JamScript/actions/runs/33945454915)

Job: [build-linux-x86_64-toolchain](https://github.com/ArcheLabs/JamScript/actions/runs/33945454915/job/101250590762)

The bundle A/B build, both verification passes, byte comparison, prebuilt CLI
preparation, and external fixture preparation all passed. The run failed only
at the final clean offline consumer build.

Root cause: Cargo was building host-side build scripts and proc-macros with
the default linker name cc. The consumer PATH intentionally hides host cc, so
proc-macro2 failed before the PolkaVM guest linker phase.

Fix: pass the managed Clang and archiver explicitly through PolkaVmBuildConfig,
set CC/CXX/AR, and set the Linux x86_64 host Cargo linker to the absolute
managed Clang path. The consumer PATH remains restricted.

The hosted UI exposed only the step failure and smoke log path; the supplied
smoke output confirmed the cc ENOENT boundary.

## Next Hosted Run

The next run will verify the managed host linker fix with host cc, Rust, Cargo,
LLVM, Node, and ScriptC executables hidden from PATH.
