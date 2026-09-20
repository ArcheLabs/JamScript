<div align="center">
  <img src="https://docs.minijam.xyz/zh-CN/img/logo.svg" width="96" alt="MiniJAM logo" />

  # JamScript

  **以接近 TypeScript 的开发体验构建 JAM 服务。**

  [English](README.md) · [简体中文](README.zh-CN.md) · [文档](https://docs.minijam.xyz/zh-CN/docs/jamscript)

  ![Release](https://img.shields.io/github/v/release/ArcheLabs/JamScript?include_prereleases&sort=semver)
  ![License](https://img.shields.io/github/license/ArcheLabs/JamScript)
</div>

JamScript 将 JAM/PVM 的底层复杂度封装在一套简洁的语言、确定性构建流程和 `jams` CLI 中。开发者专注于服务逻辑，JamScript 负责编译工具链、PVM 产物、部署流程以及本地 Backend。

## ⚡ 安装

```bash
curl -fsSL https://install.minijam.xyz/jamscript | bash
```

安装器会自动选择最新发布的 JamScript 版本，并安装：

- `jams` — JamScript CLI
- 受 JamScript 管理的编译工具链
- 与当前 JamScript 版本匹配的原生 `jamscript-service-backend`

当前支持：**Linux x86_64** 与 **macOS Apple Silicon**。

如需固定版本：

```bash
curl -fsSL https://install.minijam.xyz/jamscript \
  | bash -s -- --version v0.1.0-rc.7
```

## 🧩 示例

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

构建并部署：

```bash
jams build
jams deploy --network local
```

应用需要本地 Backend 时：

```bash
jams backend start --network local
```

## ✨ JamScript 为你处理

- 确定性的 JamScript → PVM 构建
- 编译器与工具链自动管理
- JAM 兼容的类型 ABI 与托管状态
- Ownership 所有权授权
- MiniJAM 部署
- 通过 `jams` CLI 管理本地 Backend

正常应用开发无需直接处理 Refine / Accumulate 等底层细节。

## 📚 文档

语言、架构、部署与示例请查看：

**[JamScript 文档 →](https://docs.minijam.xyz/zh-CN/docs/jamscript)**

## ⚠️ 当前限制

JamScript `v0.1` 目前仍是 RC / 测试网开发者预览版本。

- 当前实现使用成熟的 PolkaVM 工具链。这提供了可靠的执行基础，但也带来了一定的效率损失；随着工具链进一步针对 JamScript 优化，我们会逐步降低这部分开销。
- 当前 ScriptC 执行路径中，部分普通数值计算默认仍通过浮点 `number` 表示。服务边界上的 `u64`、`u128` 等定宽 ABI 类型仍然是明确的，但内部数值 lowering 尚未完全优化；正式版本会重点优化这一问题。
- 当前版本暂不支持 Windows。
- 通用 JAM 主网部署仍属于后续工作。

## 📄 License

Apache-2.0
