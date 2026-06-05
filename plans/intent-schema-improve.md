## Plan: AgentsVillage Schema-Control Architecture

本计划不再把目标定义为“把 Discord handler 拆干净”或“把运行时请求包一层 Intent enum”。这些工作已经完成了最关键的第一阶段。接下来的目标，是在当前 AgentsVillage 系统内部，把 `Schema` 提升为长期的控制面真相源，让 `Engine` 通过读取 Schema 来决定允许做什么、如何组装上下文、如何记录审计，以及哪些安全边界必须强制执行。

这里有一个明确的边界：

- 本计划 **包含**：在当前系统内定义并执行一份可读、可持久化、可审计的 agent schema。
- 本计划 **不包含**：把“人类自然语言 intent”自动编译成 schema。那是另一个系统的问题，不属于当前 AgentsVillage 的实现范围。

换句话说，当前项目的任务不是做“intent-to-schema compiler”，而是先把 schema 真正变成一个可执行、可审查的控制平面。

**Core Thesis**
1. `Intent` 继续保留，但它只负责运行时请求分类，例如 `Chat`、`ResetSession`、`Command`。
2. 长期真相源不是 `Intent` enum，而是 agent 的 declarative schema。
3. `ExecutionEngine` 不应主要依赖硬编码分支来决定行为；它应读取 schema 后执行，并在必要时拒绝执行。
4. `EventLog` 不是附属日志，而是审计面。它应与 schema 中声明的 audit requirements 对齐。
5. `role.md`、`memory.md`、`sessions/*.md` 目前仍可保留为内容源或兼容存储，但“何时读、何时写、何时摘要、哪些操作允许发生”应逐步由 schema 控制。

## Current Position

当前代码已经具备以下基础：

- 单一路径：Discord → `AppRequest` → `Intent` → `AppService` → `ExecutionEngine` → `AppResponse`
- 每个 agent 已有 `agent.yaml`、`capabilities.yaml`、`state.yaml` 和 `events.jsonl`
- `SchemaStore`、`EventLog`、`SchemaRenderer` 已存在
- `ChatExecutor` 已将聊天、副作用和事件记录集中到 engine 层

但距离“schema 驱动系统行为”还有明显差距：

- schema 当前主要承载 identity、capabilities 和少量 state，不足以完整表达行为规则
- `ExecutionEngine` 的执行行为仍主要硬编码在 `match intent` 中
- `capabilities` 当前更像弱配置，尚未形成严格 gate
- 审计事件虽然存在，但还不是“由 schema 定义必须记录什么”
- 安全边界尚未从 schema 中读取并强制执行

## Non-Goals

以下内容明确不在本计划内：

1. 把用户自然语言、产品需求或人类高层 intent 自动编译成 schema
2. 设计一个通用的 workflow DSL 或跨系统 orchestration engine
3. 让 schema 直接生成 Rust 代码或替代实现代码
4. 在本阶段移除 `role.md`、`memory.md` 或 `sessions/*.md` 这些现有文件

## Target Model

本计划完成后的目标模型是：

1. 平台 adapter 产生运行时请求：`AppRequest`
2. `IntentCompiler` 只负责把请求归类为 `Intent`
3. `ExecutionEngine` 加载 agent schema
4. Engine 根据 schema 做三类判断：
   - 这个 intent 是否允许执行
   - 这个 intent 应按什么规则执行
   - 这个 intent 必须留下哪些审计记录
5. Renderer 根据 schema 规定的 prompt/memory policy 组装上下文
6. Executor 执行副作用并写入事件
7. 后续任何行为、审计检查和安全分析，都尽量优先从 schema + event log 读取，而不是从散落代码里反推

这里需要强调：`Intent` 是短生命周期的 runtime token，`Schema` 才是长期存在、可审查、可持久化的控制描述。

## Minimal Executable Schema

当前项目需要的不是一个“能表达一切”的 schema，而是一份最小可执行 schema。它只需要覆盖当前系统已经存在的行为面：聊天、清会话、命令、上下文窗口、摘要、prompt 组装、审计记录和安全 gate。

### Logical Shape

逻辑上，这份 schema 应至少包含六个块：

1. `identity`
2. `intent_policy`
3. `prompt_policy`
4. `memory_policy`
5. `audit_policy`
6. `state`

可以继续沿用当前多文件布局，但逻辑上应视为一个聚合 schema：

- `agent.yaml` 承载 `identity` 与 agent-level policy 入口
- `capabilities.yaml` 承载 `intent_policy`、`memory_policy`、`audit_policy`、`safety_policy`
- `state.yaml` 承载 mutable runtime state

### Minimal Fields

下面是一版适合当前 AgentsVillage 的最小 schema 结构。它不尝试表达任意 workflow，只表达当前系统已经在执行的 agent 行为。

```yaml
# agent.yaml
schema_version: 1
identity:
  id: discord-695909901129482280-1505193999491792946
  display_name: agentsvillage-main-goldenmac

prompt_policy:
  role_source: role.md
  memory_source: memory.md
  include_long_term_memory: true

# capabilities.yaml
intent_policy:
  allow_chat: true
  allow_reset_session: true
  command_mode: reject_unknown

memory_policy:
  context_window: 20
  summarize_old_messages: true
  write_summary_to_memory: true

audit_policy:
  emit_events:
    - chat_started
    - chat_completed
    - chat_failed
    - reset_session
    - memory_summarized
  persist_session_transcript: true

safety_policy:
  deny_disabled_intents: true
  require_explicit_user_command_for_reset: true

# state.yaml
state:
  event_cursor: 0
  last_reset_at: null
  last_summary_at: null
```

### Semantics

这些字段的语义必须是可执行的，而不只是“给 prompt 看”的注释：

- `intent_policy.allow_chat`
  Engine 在执行 `Intent::Chat` 之前强制检查。若为 `false`，直接返回拒绝响应。

- `intent_policy.allow_reset_session`
  Engine 在执行 `Intent::ResetSession` 前强制检查。若为 `false`，不允许 `/new` 生效。

- `intent_policy.command_mode`
  定义命令处理策略。当前最小值可以只有 `reject_unknown`，后续再扩展 allowlist。

- `prompt_policy.role_source` / `memory_source`
  不要求把 prompt 文本搬进 schema，但 schema 必须声明 prompt 从哪里读，以及读不读 long-term memory。

- `memory_policy.context_window`
  这是执行参数，不应只存在于全局配置或硬编码；engine 应优先读取 schema 中的值。

- `memory_policy.summarize_old_messages`
  不是 prompt hint，而是 runtime switch。若为 `false`，engine 不得触发摘要。

- `memory_policy.write_summary_to_memory`
  控制摘要是否允许写回 `memory.md` 或其后续替代存储。

- `audit_policy.emit_events`
  声明该 agent 哪些事件是“必须存在”的审计记录。执行器或验证器可用它检查遗漏事件。

- `audit_policy.persist_session_transcript`
  声明是否保留 `sessions/*.md` 作为 transcript。

- `safety_policy.deny_disabled_intents`
  一旦 capability 关闭，engine 必须拒绝执行，而不是仅在 prompt 里提示 LLM。

- `safety_policy.require_explicit_user_command_for_reset`
  确保 reset 只能由显式 `/new` 触发，而不是由普通聊天内容诱导触发。

- `state.*`
  只承载运行时可变信息，不承载规范本身。也就是说，`state` 是 engine 更新的，policy 是人或上游系统声明的。

## What This Schema Is Not

为了避免歧义，明确说明这份 schema 当前还不是：

- 不是用户自然语言意图的完整表达
- 不是任意 DAG workflow 定义
- 不是“只看 schema 就能合成全部业务代码”的规范
- 不是 LLM system prompt 的全文替代品

它是一个更现实的中间层：

- 让 engine 按声明的 policy 运行
- 让行为边界和审计要求可读、可检视
- 为未来更强的 intent-first 系统保留接口

## Implementation Principles

1. 先把 schema 从“metadata”变成“control surface”，再考虑更高级的 schema generation。
2. 先让 engine 真正读取 schema 并 enforce，再增加 schema 字段。
3. 所有安全与审计相关字段都必须有明确执行语义，不能只用于 prompt 描述。
4. `Intent` 与 `Schema` 分工明确：Intent 是 runtime dispatch；Schema 是 long-lived policy。
5. schema 设计优先服务当前系统的有限行为面，不提前为通用 workflow 过度抽象。

## Steps

### Phase 0: Freeze the New Baseline

目标：承认当前系统已经完成“统一请求路径”的第一阶段，并以此为后续重构基线。

1. 将当前 `AppRequest -> Intent -> AppService -> ExecutionEngine` 路径视为新基线，不再把“分层架构本身”作为主要迁移目标。
2. 在文档中明确：当前缺口不在 handler，而在 schema 没有成为行为真相源。
3. 保留现有 `role.md`、`memory.md`、`sessions/*.md` 作为内容源与兼容存储。

### Phase 1: Promote Schema from Metadata to Policy

目标：扩展当前 schema 结构，使其足以表达当前 engine 所需的执行规则。

1. 在 `src/app/schema.rs` 中将当前 schema 扩展为最小可执行 policy 结构。
2. 在 `SchemaStore` 中支持这些 policy 字段的初始化与读写。
3. 保持多文件布局，但在代码中引入一个逻辑聚合视图，例如 `ResolvedAgentSchema`。
4. 明确哪些字段是 declarative policy，哪些字段是 mutable state。

### Phase 2: Make the Engine Read Schema Before Execution

目标：让 `ExecutionEngine` 不再只靠硬编码分支，而是在执行前读取并检查 schema。

1. 在处理 `Intent::Chat` 前检查 `intent_policy.allow_chat`。
2. 在处理 `Intent::ResetSession` 前检查 `intent_policy.allow_reset_session`。
3. 在处理 `Intent::Command` 时根据 `command_mode` 决定拒绝或 allowlist 行为。
4. 将 `memory_policy.context_window` 作为 engine 的首要来源。
5. 将 `memory_policy.summarize_old_messages` 变成真正的 runtime gate。

### Phase 3: Move Prompt Assembly Under Schema Policy

目标：不立即把 prompt 内容搬进 schema，但让 schema 决定 prompt 如何组装。

1. `SchemaRenderer` 不再只是“role + memory 的拼接器”，而是根据 `prompt_policy` 决定读哪些源。
2. `include_long_term_memory` 为 `false` 时，renderer 不得读取 `memory.md`。
3. 后续若引入更多 prompt source，也必须先在 schema 中声明。

### Phase 4: Make Audit Requirements Explicit

目标：让 event log 从“默认会写”变成“按 schema 要求必须写”。

1. 执行器按 `audit_policy.emit_events` 写入事件。
2. 补一个验证器或测试助手，检查某个 intent 执行后是否满足 schema 要求的事件集合。
3. `persist_session_transcript` 为 `false` 时，为未来停写 `sessions/*.md` 留出路径。

### Phase 5: Turn Safety from Prompt Hint into Runtime Guard

目标：把当前 capability 和 future safety policy 从“提示 LLM”升级为“engine 硬约束”。

1. `deny_disabled_intents` 为 `true` 时，engine 必须在 LLM 调用前直接拒绝。
2. reset 只能由明确命令触发，不允许普通聊天内容间接触发。
3. 总结写回 `memory.md` 需要经过 `memory_policy.write_summary_to_memory` 开关。

### Phase 6: Tighten Docs and Verification Around Schema Truth

目标：让架构文档、测试与未来审计都围绕 schema 展开。

1. 更新架构文档，明确 `Intent` 与 `Schema` 的不同职责。
2. 更新 README，明确当前系统不是 intent-to-schema compiler。
3. 写清楚 schema 字段的执行语义，而不是只写文件名与结构。

## Relevant Files

- `src/app/compiler.rs`
  继续作为 runtime `Intent` 分类器，不承担 schema 生成职责。

- `src/app/schema.rs`
  需要从“identity/capability/state 容器”升级为最小可执行 policy 模型。

- `src/app/executor.rs`
  需要从“match intent 然后直接执行”升级为“先读 schema，再判断是否执行以及如何执行”。

- `src/app/renderer.rs`
  需要从简单拼 prompt 升级为受 `prompt_policy` 控制的 renderer。

- `src/agent/schema_store.rs`
  需要支持更丰富的 policy 字段和逻辑聚合读取。

- `src/agent/event_log.rs`
  继续作为审计记录载体，后续应与 `audit_policy` 对齐。

- `src/agent/model.rs`
  需要更明确地区分 schema-backed policy 与 mutable state。

- `workspace/{agent_id}/schema/*.yaml`
  是长期控制面的主要落点。

## Known Gaps To Address

1. 当前 `capabilities` 没有形成严格 runtime gate，仍带有“弱配置”特征。
2. 当前 `SchemaRenderer` 只部分读取 schema，还未真正受 prompt policy 驱动。
3. 当前审计事件是代码里固定写入的，而不是由 schema 声明 required events。
4. 当前安全边界主要靠代码约定，还没有形成 schema-driven guard。
5. `MemoryManager` 仍有历史 IO 与 markdown parsing 债，需要在 schema control surface 稳定后继续收缩。

## Verification

1. Schema policy tests
   - `allow_chat = false` 时，`Intent::Chat` 被 engine 拒绝。
   - `allow_reset_session = false` 时，`Intent::ResetSession` 被拒绝。
   - `summarize_old_messages = false` 时，不产生 `memory_summarized` 事件。

2. Renderer tests
   - `include_long_term_memory = false` 时，system prompt 不读取 `memory.md`。
   - 修改 `role_source` 或 `memory_source` 时，renderer 按 schema 路径解析。

3. Audit tests
   - 执行聊天后，事件集合满足 `audit_policy.emit_events` 的声明。
   - 当某个事件被关闭或新增时，测试能立刻暴露不一致。

4. Safety tests
   - 非显式命令路径不能触发 reset。
   - 被 schema 禁止的 intent 不进入 LLM 调用。

5. End-to-end tests
   - 普通消息回复仍正常
   - `/new` 仍正常
   - 长上下文摘要行为受 schema 开关控制
   - 事件日志与 session transcript 持久化符合 schema 声明

## Decisions

1. 保留 `Intent`，但降级其职责：它是 runtime dispatch token，不是长期真相源。
2. 保留多文件 YAML 布局，而不是强行合并成一个大文件：当前 workspace 结构已存在，渐进演进成本更低。
3. 保留 `role.md`、`memory.md` 作为内容源，而不是立即将文本内容内嵌进 schema：当前阶段先迁移控制权，不迁移全部内容真相源。
4. 不在当前项目中引入“intent-to-schema generation”：它会混淆当前系统边界，并掩盖 schema control surface 本身还未落地的问题。
5. 让审计与安全先从最小可执行规则开始，不提前设计通用 workflow language。

## References

- https://medium.com/@dqj1998/intent-first-programming-the-next-evolution-of-presupposition-f01e0603eb77?sk=568f346089c32fc48b4c04d04829a941