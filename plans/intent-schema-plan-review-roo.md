# Intent-Schema 迁移计划审查报告

> 基于对原计划 `intent-schema-improve.md` 与实际代码库的对照分析

---

## 一、原计划的正确判断

原计划准确识别了核心问题：

- [`src/discord/handler.rs`](../src/discord/handler.rs:71) 的 `message()` 方法直接编排了 `MemoryManager`、`build_llm_messages`、`LlmClient.chat` 和响应格式化，职责严重过载。
- [`src/discord/router.rs`](../src/discord/router.rs:9) 混合了平台辅助函数（`build_agent_id`、`split_message`）与业务逻辑（`build_llm_messages`、摘要生成），边界不清。
- 10 步分阶段迁移的整体思路是合理的。

---

## 二、发现的问题与改进建议

### 问题 1：步骤 1-3 过于抽象，形成不必要的阻塞

**现状：** 原计划将 Phase 1 的三个步骤全部设计为"先设计、后实现"的阻塞节点。但整个代码库只有约 700 行，完全可以从代码直接推导出接口，不需要三步纯设计。

**改进建议：** 将步骤 1-3 合并为一个"定义核心类型"步骤，并直接给出 Rust 模块布局：

```
src/app/
  mod.rs          — AppRequest, AppResponse, Intent 枚举
  service.rs      — AppService::handle()
  compiler.rs     — IntentCompiler trait + 默认实现
  executor.rs     — ExecutionEngine trait + LegacyChatExecutor
```

---

### 问题 2：`Intent` 枚举的 payload 未定义

**现状：** 步骤 5 列出了 `Chat`、`ResetSession`、`SummarizeMemory`、`Clarify`、`Command` 五类 intent，但没有说明每个 intent 携带什么数据。

对照实际代码：
- [`build_llm_messages()`](../src/discord/router.rs:9) 需要：`agent`、`memory_manager`、`llm_client`、`context_window` → 对应 `Chat` 的执行上下文
- [`clear_today_session()`](../src/agent/memory.rs:151) 只需要 workspace 路径 → 对应 `ResetSession`
- [`router.rs:25-58`](../src/discord/router.rs:25) 的摘要生成当前**内嵌在** `build_llm_messages` 内部，是隐式触发的，不是用户发起的 intent

**改进建议：** 在计划中明确 payload 定义：

```rust
enum Intent {
    Chat { user_text: String, author: String },
    ResetSession,
    SummarizeMemory { messages: Vec<SessionMessage> },  // 由 executor 内部触发，非用户 intent
    Clarify { question: String },
    Command { name: String, args: Vec<String> },
}
```

同时需要澄清：`SummarizeMemory` 应由执行器在 `total > context_window` 时内部触发，而不是作为用户可见的 intent。原计划将内部触发与用户发起的 intent 混淆了。

---

### 问题 3：`CONTEXT_WINDOW` 硬编码未被计划覆盖

**现状：** [`handler.rs:19`](../src/discord/handler.rs:19) 有 `const CONTEXT_WINDOW: usize = 20` 硬编码。这是一个策略决策，应属于 Schema 或配置层，而不是 Discord adapter。原计划步骤 6 提到将"上下文窗口逻辑"从 `router.rs` 移出，但没有提到 `handler.rs` 中的这个常量。

**改进建议：** 在步骤 6 中明确将 `context_window` 移入 `AgentSchema` 或 `GlobalConfig` 作为可配置字段，由执行器从配置中读取。

---

### 问题 4：`SchemaStore` 与 `MemoryManager` 的拆分边界模糊

**现状：** 步骤 6 说要将 `MemoryManager` 从"Markdown transcript manager"升级为"Schema + Event store facade"，同时新增 `schema/*.yaml` 和 `events/*` 的 API。这让一个结构体承担两种截然不同的职责。

查看 [`memory.rs`](../src/agent/memory.rs:8)，`MemoryManager` 已经做了太多事：session 追加、session 读取、memory 读取、memory 写入、session 清除、消息计数。

**改进建议：** 明确拆分为三个独立类型：

| 类型 | 职责 | 文件 |
|------|------|------|
| `SessionStore`（或保留 `MemoryManager` 作为兼容层） | 只处理 `sessions/*.md` 和 `memory.md` | `src/agent/memory.rs` |
| `SchemaStore` | 处理 `schema/*.yaml` 的读写 | `src/agent/schema_store.rs` |
| `EventLog` | append-only JSONL 写入器 | `src/agent/event_log.rs` |

---

### 问题 5：`AgentManager` 内存缓存失效问题未被处理

**现状：** [`manager.rs`](../src/agent/manager.rs:11) 将 agent 缓存在 `HashMap<String, Agent>` 中，由 `Mutex` 保护。当新的 `SchemaStore` 写入 `schema/*.yaml` 时，内存中的 `Agent` 结构体（持有从 `role.md` 加载的 `role: String`）将变为过时数据。原计划没有处理内存缓存与磁盘 schema 之间的一致性问题。

**改进建议：** 在计划中明确缓存失效策略，三选一：
1. 让 `Agent` 持有 `SchemaStore` 引用，每次请求时 read-through
2. 在 `AgentManager` 上新增 `reload_agent()` 方法
3. 让 `Agent` 不可变，每次请求都从 `SchemaStore` 重新加载

---

### 问题 6：`build_agent_id` 的迁移目标不明确

**现状：** 步骤 7 说"`build_agent_id` 这类平台标识逻辑可以保留在 adapter 层"，但 [`build_agent_id`](../src/discord/router.rs:120) 目前在 `router.rs` 中，而 `router.rs` 本身就是要被拆分的对象。计划没有说明它具体移到哪个文件。

**改进建议：** 明确指定：`build_agent_id`、`get_thread_id`、`get_channel_name`、`split_message`、`current_timestamp` 这五个函数全部移入新建的 `src/discord/adapter.rs` 模块，或作为 `DiscordAdapter` 结构体的方法。

---

### 问题 7：`AppResponse` 没有错误变体定义

**现状：** 原计划提到所有输出都收敛为 `AppResponse`，但没有定义其变体。查看 [`handler.rs:138-157`](../src/discord/handler.rs:138)，当前错误处理直接在 handler 内联生成 Discord 消息。这部分逻辑需要有明确的归属。

**改进建议：** 在步骤 2 中明确定义 `AppResponse`：

```rust
enum AppResponse {
    Text(String),          // 普通回复
    Ephemeral(String),     // 仅发起者可见（如 /new 确认）
    Error(String),         // 用户可见的错误信息
    Silent,                // 无需回复（如预创建 agent）
}
```

并明确 Discord adapter 负责将 `AppResponse::Error` 映射为当前的错误消息模式。

---

### 问题 8：验证章节缺乏测试基础设施规划

**现状：** 验证步骤提到了 `cargo test`，但当前代码库**零测试**。计划说"补单元测试"，但没有说明如何 mock `LlmClient` 和 `MemoryManager`。

**改进建议：** 补充具体的测试策略：
- 将 `LlmClient` 包装为 trait（或使用 trait object），使其可被 mock
- 将 `SchemaStore` 定义为 trait，测试时使用内存实现
- 至少指定一个集成测试：将 `Chat` intent 送入 `AppService`，验证输出与旧路径一致

---

### 问题 9：typing indicator 的归属未被处理

**现状：** [`handler.rs:145`](../src/discord/handler.rs:145) 在 LLM 调用**之前**调用 `msg.channel_id.broadcast_typing()`。这是一个 Discord 专属的副作用，发生在将来会变成 `AppService::handle()` 的流程中间。原计划没有说明这个调用应该放在哪里。

**改进建议：** 明确说明：Discord adapter 在调用 `AppService::handle()` **之前**发送 typing indicator，而不是在 `AppService` 内部处理。这是 adapter 层的职责，不应渗入应用服务层。

---

### 问题 10：`build_llm_messages` 的隐式写副作用未被标记为重构风险

**现状：** [`build_llm_messages`](../src/discord/router.rs:9) 内部在 `total > context_window` 时会调用 `memory_manager.append_memory()`（摘要写入）。这是一个**隐藏在读操作中的写副作用**。步骤 9 说要将其包装为 `LegacyChatExecutor`，但没有标记这个风险。

**改进建议：** 在步骤 9 中明确指出：在将 `build_llm_messages` 包装为 `LegacyChatExecutor` 之前，必须先将摘要触发逻辑提取为独立的显式步骤。执行器应将摘要生成作为单独的、可观测的操作调用，而不是作为消息构建的副作用。

---

## 三、改进汇总表

| # | 问题 | 严重程度 | 改进方向 |
|---|------|----------|----------|
| 1 | 步骤 1-3 过于抽象，阻塞实现 | 中 | 合并为一步，给出具体模块布局 |
| 2 | Intent payload 未定义 | **高** | 在步骤 5 中补充 payload 定义，区分用户 intent 与内部触发 |
| 3 | `CONTEXT_WINDOW` 硬编码未覆盖 | 中 | 纳入步骤 6，移入配置层 |
| 4 | `SchemaStore` 与 `MemoryManager` 边界模糊 | **高** | 明确拆分为三个独立类型 |
| 5 | Agent 缓存失效策略缺失 | **高** | 在步骤 3/6 中明确缓存一致性方案 |
| 6 | `build_agent_id` 迁移目标不明 | 低 | 明确命名目标文件 `src/discord/adapter.rs` |
| 7 | `AppResponse` 缺少错误变体 | **高** | 在步骤 2 中定义完整枚举 |
| 8 | 测试基础设施无规划 | 中 | 补充 trait mock 策略 |
| 9 | typing indicator 归属未定 | 低 | 明确为 adapter 层职责，在 `AppService` 调用前执行 |
| 10 | `build_llm_messages` 隐式写副作用未标记 | **高** | 在步骤 9 中明确提取摘要逻辑为前置步骤 |

---

## 四、建议的修订后步骤顺序

```
Phase 1（基础抽象）
  Step 1: 定义核心类型 + 模块布局（合并原 1-3）
          - 创建 src/app/ 目录结构
          - 定义 Intent enum（含 payload）
          - 定义 AppRequest / AppResponse（含错误变体）
          - 定义 IntentCompiler / ExecutionEngine trait
          - 定义 SchemaStore / EventLog / SessionStore 三个独立类型

Phase 2（新核心实现）
  Step 2: 实现 AppService::handle() 骨架（原 Step 4）
  Step 3: 实现 LegacyChatExecutor，提取 build_llm_messages 的写副作用（原 Step 5 + 9 前半）
  Step 4: 重构持久化层，拆分 MemoryManager（原 Step 6）
          - 同步将 CONTEXT_WINDOW 移入配置层

Phase 3（接入与降级）
  Step 5: 将 Discord 降级为 adapter（原 Step 7）
          - 新建 src/discord/adapter.rs
          - typing indicator 移至 AppService 调用前
  Step 6: 升级 Agent 模型，明确缓存失效策略（原 Step 8）

Phase 4（收尾）
  Step 7: 切断旧主路径（原 Step 9 后半）
  Step 8: 补测试基础设施（trait mock + 集成测试）（原 Verification）
  Step 9: 更新文档（原 Step 10）
```

---

## 五、不需要改动的部分

以下原计划内容判断正确，无需修改：

- 双轨迁移策略（先建新核心，再接入现有流量）
- YAML 作为 schema 主格式，JSONL 作为事件日志格式
- `role.md` / `memory.md` 保留为兼容层而非立即下线
- `merge_agent_config` 的现有分层模式可复用
- 首期 intent 枚举空间保持小而稳的原则
- MCP / Web 前端扩展点的预留思路
