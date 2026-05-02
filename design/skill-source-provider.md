# Provider Abstraction Design

## Problem

当前 skill 来源逻辑与 GitHub 深度耦合，需要引入 provider 抽象层，使 install / sync / update 流程能透明地支持多个来源（GitLab 等）。

## Scope

**In:**
- 定义 `SkillProvider` trait 及 `ProviderRegistry`
- 将现有 `cli/github.rs` 重构为 `GitHubProvider` 实现
- 将 `install` / `sync` 流程改为通过 registry 路由
- 迁移共享数据类型（`InstallPlan`, `ResolvedSkill`, `ExtractTarget`）

**Out:**
- GitLab provider 的具体实现
- 修改 `skills.toml` 文件格式 / `SkillEntry` 已有字段
- 修改 CLI 接口

## Assumptions

- `SkillEntry.slug`（owner/repo 格式）和 `sha`、`path` 对 GitLab 同样适用，无需新字段
- `source_url` 已存入 `SkillEntry`，足以在 sync 时通过 `handles()` 找回对应 provider，不需要额外存储 `provider = "github"` 字段
- GitHub tarball URL 无过期问题，可以在运行时由 `(slug, sha)` 重建
- 仅支持 HTTP-based provider（GitHub、GitLab、私有 HTTP 服务）

## Components

| 组件 | 职责 |
|------|------|
| `SkillProvider` trait | 定义 provider 必须实现的三个操作：URL 匹配、解析安装计划、拉取并解压 |
| `ProviderRegistry` | 持有所有已注册 provider，根据 URL 路由到正确的 provider |
| `GitHubProvider` | 封装现有 `cli/github.rs` 全部逻辑，实现 `SkillProvider` |
| `providers::types` | 共享数据结构：`InstallPlan`、`ResolvedSkill`、`ExtractTarget` |

## Interfaces

```rust
// src/providers/mod.rs

pub trait SkillProvider: Send + Sync {
    /// 判断此 provider 是否能处理该 URL（纯字符串匹配，不做网络请求）
    fn handles(&self, url: &str) -> bool;

    /// 将用户输入的 URL 解析为安装计划（含 archive_url 和所有待安装 skill）
    /// 失败时返回解析/网络错误；URL 语法合法但找不到资源返回 PathNotFound
    fn resolve_install_plan(&self, url: &str) -> SkillsResult<InstallPlan>;

    /// 下载 archive_url 并将各 target 路径解压到对应目录
    /// archive_url 由本 provider 生成，对调用方不透明
    fn fetch_and_extract(&self, archive_url: &str, targets: &[ExtractTarget]) -> SkillsResult<()>;

    /// 由已存储的 SkillEntry 重建 archive_url，供 sync 时复用已锁定的 SHA
    fn archive_url_for_entry(&self, entry: &SkillEntry) -> String;
}

pub struct ProviderRegistry { /* ... */ }

impl ProviderRegistry {
    pub fn new(providers: Vec<Box<dyn SkillProvider>>) -> Self;

    /// 返回能处理该 URL 的第一个 provider；找不到时返回 UnsupportedProvider 错误
    pub fn get(&self, url: &str) -> SkillsResult<&dyn SkillProvider>;
}
```

调用侧示意：

```rust
// install.rs
pub fn install_skill(url: &str, base_dir: &Path, yes: bool,
                     registry: &ProviderRegistry) -> SkillsResult<()> {
    let provider = registry.get(url)?;
    let plan = provider.resolve_install_plan(url)?;
    install_plan(provider, plan, base_dir, yes)
}

// sync.rs - 仅用 provider 重建 archive_url，不重新解析 ref
fn sync_one(provider: &dyn SkillProvider, entry: &SkillEntry, ...) {
    let archive_url = provider.archive_url_for_entry(entry);
    provider.fetch_and_extract(&archive_url, &[target])?;
}
```

## Data Model

**不变**：`SkillEntry`、`SkillsConfig` 保持现有结构，`source_url` 天然充当 provider 路由键。

**迁移**：以下类型从 `cli/github.rs` 移到 `providers/mod.rs`，成为 provider 无关的公共类型：

```
InstallPlan     { archive_url, is_batch, skills: Vec<ResolvedSkill> }
ResolvedSkill   { name, source_url, slug, sha, path }
ExtractTarget   { path, dest_dir }
```

**`GitHubProvider`** 持有 `ureq::Agent`（包含 proxy 配置、token），作为结构体内部状态，不暴露出去。

目标文件结构：

```
src/
  providers/
    mod.rs        # SkillProvider trait, ProviderRegistry, 共享类型
    github.rs     # GitHubProvider 实现（从 cli/github.rs 迁移）
  cli/
    mod.rs
    install.rs    # 接收 &ProviderRegistry
    sync.rs       # 接收 &ProviderRegistry
    update.rs     # 不变
    uninstall.rs  # 不变
    list.rs       # 不变
    prompt.rs     # 不变
  models.rs
  errors.rs
  utils.rs
  main.rs         # 构造 ProviderRegistry
```

## Load-Bearing Decisions

**1. Provider 识别通过 `source_url` 推断，不存入 `skills.toml`**
- 选择：`handles(url)` 匹配，而非存储 `provider = "github"`
- 代价：若 URL 格式有歧义（两个 provider 都 `handles` 同一 URL），按注册顺序取第一个；实践中 GitHub / GitLab URL 前缀天然不同，不会冲突

**2. `archive_url` 对调用方不透明**
- 选择：调用方只传递字符串，由生成它的 provider 负责 fetch
- 代价：`archive_url` 必须与对应 provider 一起流转；在当前设计中 plan 由 provider 生成、也由同一 provider fetch，天然满足

**3. HTTP client 封装在 Provider 内部**
- 选择：`GitHubProvider::new()` 内部构造 `ureq::Agent`（读取 proxy/token 环境变量）
- 代价：provider 构造有副作用（读取环境变量）；但这比把 agent 从外部传入再在 trait 里强转要干净得多

**4. `SkillEntry` schema 不变**
- 选择：`slug` 复用（对 GitLab 语义相同，均为 `owner/repo`），`sha` 和 `path` 通用
- 代价：如果将来出现 slug 格式完全不同的 provider，可能需要引入 `metadata: HashMap<String, String>` 逃生口——但先不做

## Risks / Open Questions

1. **`archive_url_for_entry` 对 GitLab 的字段够用吗？** GitLab archive 格式为 `/api/v4/projects/{encoded_slug}/repository/archive.tar.gz?sha={sha}`，现有 `slug` 和 `sha` 字段足以重建。低风险。

2. **batch install 时 `archive_url` 跨 skill 共享**：目前 GitHub batch 用同一个 tarball（repo 级别），解压多个路径。GitLab 行为相同，但若某个 provider 需要每个 skill 单独下载，`InstallPlan` 的 `archive_url` 字段需要移到 `ResolvedSkill` 里。现在先保留在顶层，遇到再调整。

3. **`UnsupportedProvider` 错误类型缺失**：`errors.rs` 需要新增一个变体。改动小，不是风险。

## Implementation Roadmap

- [x] **[迁移类型]** 新建 `src/providers/mod.rs`，将 `InstallPlan` / `ResolvedSkill` / `ExtractTarget` 从 `cli/github.rs` 移入
  - Purpose: 建立公共类型基础，后续 trait 和 impl 都依赖它
  - Verification: `cargo check` 无报错

- [x] **[定义接口]** 在 `src/providers/mod.rs` 中声明 `SkillProvider` trait 和 `ProviderRegistry`；在 `errors.rs` 新增 `UnsupportedProvider` 变体
  - Purpose: 锁定调用侧约定，后续两端可以独立开发
  - Verification: `cargo check`

- [x] **[实现 GitHubProvider]** 新建 `src/providers/github.rs`，将 `cli/github.rs` 的逻辑包装进 `struct GitHubProvider(ureq::Agent)` 实现 trait
  - Purpose: 验证 trait 设计能完整表达现有 GitHub 流程
  - Verification: `cargo test` 全绿，行为与重构前一致

- [x] **[接入 CLI]** `install.rs` / `sync.rs` 接收 `&ProviderRegistry`，通过 `registry.get(url)` 路由；`main.rs` 构造 `ProviderRegistry::new(vec![Box::new(GitHubProvider::new())])`
  - Purpose: 打通端到端流程
  - Verification: `cargo run -- install <github-url>` 和 `sync` 手动验证

- [x] **[清理]** 删除 `cli/github.rs`，更新 `cli/mod.rs` 导出，将 `GitHubUrl` / `GitHubUrlSpec` 从 `models.rs` 移入 `providers/github.rs`
  - Purpose: 消除旧代码，避免两份逻辑并存；GitHub 相关模型收敛到 provider 模块内
  - Verification: `cargo test && cargo clippy`
