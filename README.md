<div align="center">
  <img src="https://docs.minijam.xyz/zh-CN/img/logo.svg" width="96" alt="MiniJAM logo" />

  # JamScript

  **Build JAM services with a TypeScript-like developer experience.**

  [English](README.md) · [简体中文](README.zh-CN.md) · [Documentation](https://docs.minijam.xyz/zh-CN/docs/jamscript)

  ![Release](https://img.shields.io/github/v/release/ArcheLabs/JamScript?include_prereleases&sort=semver)
  ![License](https://img.shields.io/github/license/ArcheLabs/JamScript)
</div>

JamScript hides the low-level JAM/PVM plumbing behind a small language, a deterministic build pipeline, and the `jams` CLI. You write services; JamScript handles the compiler toolchain, PVM artifacts, deployment flow, and local backend.

## ⚡ Install

```bash
curl -fsSL https://install.minijam.xyz/jamscript | bash
```

The installer automatically selects the latest published JamScript release and installs:

- `jams` — the JamScript CLI
- the managed compiler/toolchain
- the matching native `jamscript-service-backend`

Supported today: **Linux x86_64** and **macOS Apple Silicon**.

To pin an exact release:

```bash
curl -fsSL https://install.minijam.xyz/jamscript \
  | bash -s -- --version v0.1.0-rc.7
```

## 🧩 Example

```typescript
import { action, wallet, u64 } from "jam";

export const increment = action({
  auth: wallet(),
  input: { value: u64 },
  execute(ctx, input) {
    return input.value + 1;
  },
});
```

Build and deploy:

```bash
jams build
jams deploy --network local
```

Run the local backend when the application needs it:

```bash
jams backend start --network local
```

For a configured testnet profile, select it at the CLI layer:

```sh
jams backend start --network testnet
```

The CLI resolves the named profile and passes concrete Node and Formal RPC
endpoints to the backend. The backend process itself is network-name agnostic.

## ✨ What JamScript handles

- deterministic JamScript → PVM builds
- managed compiler and toolchain installation
- JAM-compatible typed ABI and managed state
- Ownership-based authorization
- MiniJAM deployment
- local backend lifecycle through the `jams` CLI

You do not need to work directly with Refine/Accumulate internals for normal application development.

## 📚 Documentation

For guides, architecture, language details, deployment, and examples:

**[JamScript Documentation →](https://docs.minijam.xyz/zh-CN/docs/jamscript)**

## ⚠️ Limitations

JamScript `v0.1` is currently an RC/testnet developer preview.

- The current implementation builds on the mature PolkaVM toolchain. This gives JamScript a reliable execution foundation, but also introduces efficiency overhead that we intend to reduce as the toolchain becomes more JamScript-specific.
- In the current ScriptC execution path, ordinary numeric computation still uses floating-point `number` representation by default in some paths. Fixed-width ABI types such as `u64` and `u128` remain explicit at service boundaries, but the internal numeric lowering is not yet fully optimized. This will be improved before the stable release.
- Windows is not supported in the current release.
- Generic JAM mainnet deployment remains future work.

## 📄 License

Apache-2.0
