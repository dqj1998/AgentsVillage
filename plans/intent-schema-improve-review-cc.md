## Review: Intent-Schema Migration Plan 改善建议

针对 [intent-schema-improve.md](intent-schema-improve.md) 的审核。整体方向（让 Discord 退化为 adapter、加一层 IR）合理，但对照 ~1100 行的当前实现，这份计划**抽象密度严重过载、对真实痛点偏轻、且缺少一个能验证可行性的最小竖切**。

---

### 1. 抽象数量与代码体量严重错配

当前实现：~1100 行 Rust、单一 Discord adapter、实际行为只有 4 个（message / thread_create / ready / `/new`），LLM executor 也只有 1 个（chat + 顺手 summarize）。

而计划在 Phase 1 一次性引入：`Intent`、`IntentEnvelope`、`IntentSchema`、`SchemaState`、`CompileResult`、`ExecutionResult`、`AppRequest`、`AppResponse`、`IntentCompiler`、`SchemaStore`、`ExecutionEngine`、`PlatformAdapter` —— 共 12 个新类型/trait。其中：

- `IntentCompiler` 当前唯一的输入是「自然语言 → Chat」与「`/new` → Reset」。这是一个 2 分支的 match，不是 compiler。
- `IntentEnvelope` 与 `AppRequest` 语义重叠（一个有 platform metadata，一个携带 intent + metadata），不需要并存。
- `IntentSchema` vs `SchemaState` 在文档里没区分清楚——前者是「能力定义」，后者是「运行时状态」？建议合并为 `AgentSchema { definition, state }` 或拆得更尖锐。

**改进**：第一版只引入 3 个核心类型：`Command`（enum，等价于现在的 Intent）、`AgentState`（替代 SchemaStore + SchemaState）、`Handler` trait（替代 ExecutionEngine + Executor 两层）。`AppRequest/AppResponse` 留在 adapter 层就够了，不需要再单独建模。等到第 2 个 adapter（MCP / Web）真的进来再拆 envelope。

---

### 2. 最大的问题：没有一个「最小竖切」里程碑

10 步里前 3 步是纯设计、第 4 步建框架、第 5 步映射意图、第 6 步动持久化。**直到第 7 步 Discord adapter 改造完成之前，没有一条端到端可运行的新路径**。这意味着前 6 步都是「写完没法验证」的死库存。

**改进**：在 Phase 2 起点加一个 **Step 3.5 / Step 4.0「Walking Skeleton」**：

- 一条最小 happy path：Discord message → `AppService::handle` → `LegacyChatHandler`（包住现在的 `build_llm_messages`）→ 回 Discord。
- 不动 SchemaStore、不动持久化、不动 role.md/memory.md，**只切换调用顺序**。
- 通过 feature flag `engine = legacy | new` 决定哪条路径执行。

有这个骨架，之后每一步重构都能立刻 smoke test。没有它，第 7 步合并时就是一次大爆炸。

---

### 3. "Schema 是真相源" 与 "Chat executor 仍读 role.md/memory.md" 自相矛盾

- Step 3 写「首期以 `schema/*.yaml` 为真相源，旧 markdown 文件不再直接驱动控制流」。
- Step 5 又写「`build_llm_messages` 与 `LlmClient::chat` 保留，但仅服务于 Chat 或 SummarizeMemory 执行器」。
- `build_llm_messages` 当前直接读 `role.md` + `memory.md` 拼 system prompt（[src/discord/router.rs:16-67](src/discord/router.rs#L16-L67)）。

如果 chat executor 仍读 markdown，那 yaml 就**不是**真相源，是平行系统。需要在计划里明确：

- **方案 A**：yaml 是真相，executor 边界处把 yaml 渲染成 prompt 文本（添加 `SchemaRenderer`）；role.md/memory.md 变成 yaml 的派生导出，不参与读路径。
- **方案 B**：暂时承认 markdown 仍是 prompt 来源，yaml 只承载 capability / config / event index 这些「控制层」事实，不接管 prompt 内容。

模糊地说"yaml 是 truth、markdown 是兼容层"会导致执行期间两边都被写，又都被读，迁移末期还要做一次对账。

---

### 4. 计划没识别出代码里真正的债

仅看现有代码就能列出来的具体问题，计划一条都没点名：

| 现状 | 位置 | 问题 |
|---|---|---|
| `count_total_messages` 把所有 session 文件读一遍数 `## ` | [src/agent/memory.rs:166-195](src/agent/memory.rs#L166-L195) | 每条消息触发一次 O(全部历史) 的 IO |
| `load_recent_messages` 同样按文件读全部内容直到 limit | [src/agent/memory.rs:69-115](src/agent/memory.rs#L69-L115) | 同上，且 markdown header parser 容易被用户消息里出现的 `## ` 字面量破坏 |
| `parse_session_file` 按字符串 split 解析消息 | [src/agent/memory.rs:199-254](src/agent/memory.rs#L199-L254) | 用户内容里有 `## ` 就会错位 |
| `clear_today_session` 直接 truncate 文件 | [src/agent/memory.rs:151-163](src/agent/memory.rs#L151-L163) | 与 append-only events 的理念冲突；事件源化之后这里必须改成发 `ResetSession` 事件 |
| `build_llm_messages` 在「读上下文」函数里夹带「写 memory.md 摘要」副作用 | [src/discord/router.rs:25-58](src/discord/router.rs#L25-L58) | 读写混合，难以测试，新架构里必须拆 |
| `AgentManager` 整体 `Arc<Mutex<>>` | [src/discord/handler.rs:107](src/discord/handler.rs#L107), [189](src/discord/handler.rs#L189), [232](src/discord/handler.rs#L232) | 任意 message 都串行化 agent get；新增 SchemaStore 后必须用 per-agent lock 或 RwLock |

**改进**：在「Further Considerations」或新增「Known Tech Debt」一节，明确这些是迁移要顺手解决的硬伤——尤其 `parse_session_file` 的脆弱性和 markdown header 计数，是 SchemaStore 必须替代的核心理由，比"架构整洁"更有说服力。

---

### 5. 测试基线缺失但 Verification 假设已有

项目当前 0 个测试（`src/` 下无 `#[cfg(test)]`、无 `tests/` 目录）。Verification 第 1-3 条都写"补单元/集成测试"，但没有 **Step 0：搭测试脚手架**：

- 决定 async runtime 测试方式（`#[tokio::test]`）。
- 决定 fixture 怎么造：用 `tempfile::TempDir` 还是检入一个 `tests/fixtures/workspace-sample/`。
- LLM client 怎么 mock（现在 `LlmClient::chat` 直连，得抽 trait 或用 wiremock）。

这是先决条件，不是 Verification 阶段才做的事。

---

### 6. "Decisions" 段没有真正的决策

现在写的是：

- "本次目标是修改实现架构"——这是题目，不是决策。
- "首期主架构是 Intent-Schema、Discord 降为 adapter"——这也是题目。
- "YAML 主存储 + JSONL 事件"——这是决策，但没写**对比项**和**否决理由**。

一个真正的 Decisions 段应该长这样：

- "选 YAML 而非 JSON：因为 role.md/memory.md 现在就由人手编辑，保留可读性比机器友好更重要；否决 JSON。"
- "选 per-agent 文件而非 SQLite：当前 agent 数 < 50，文件足够；否决 sqlite 因为部署、备份、git diff 成本上升。"
- "选双轨迁移而非 fork-and-replace：因为 Discord 是用户在线流量，否决硬切；但接受迁移期间存在 legacy executor 的代码债。"
- "**否决**统一 'multi-adapter ready' 在第一版：因为 MCP / Web 无明确时间表，提前抽象会污染 trait 边界。"

---

### 7. 迁移开关与回滚未定义

「双轨迁移」需要一个具体的切换机制：

- 是 config.toml 里的 `core.engine = "legacy" | "intent_schema"` 全局开关？
- 还是 per-agent 开关（某些 agent 试点）？
- 还是 per-request（基于 channel id 灰度）？

否则"双轨"实际上不存在，回滚只能 `git revert`。

**改进**：在 Step 4 或 Step 9 明确开关位置、默认值、何时翻转、何时删除 legacy 分支的标准（例如"新路径连续 7 天无错误率回归后删除 legacy"）。

---

### 8. Phase 与依赖关系小问题

- Step 6（持久化重构）和 Step 5（intent 映射）依赖关系写的是「可并行」，但 Step 5 的 chat executor 实现要读 SchemaStore——并行只能在 trait 边界定好之后开始，不是从 Phase 2 一开始。建议加一句"前提：Step 4 的 SchemaStore trait 已冻结"。
- Step 9「逐步删除 handler 中直接依赖 MemoryManager/LlmClient 的路径」其实是 Step 7 完成的副产品。Step 9 真正独立的工作是「删除 legacy executor」，建议改写聚焦在这点。
- Step 10（文档）依赖步骤 9 完成，但其实 Step 4 的 trait 边界稳定下来就可以先写架构图——架构文档不必等到全部砍完 legacy。

---

### 总结：建议的删改

| 动作 | 内容 |
|---|---|
| **删** | 第一版去掉 `IntentEnvelope`、`CompileResult`、`ExecutionResult` 这类中间包装；合并 `IntentSchema/SchemaState` 为 `AgentSchema`；合并 `IntentCompiler/ExecutionEngine` 为单 `Handler` trait。 |
| **加** | Walking Skeleton 里程碑（legacy executor 跑通新框架）；测试脚手架步骤；feature flag / 切换开关；明确"yaml 是不是真相源"的语义边界。 |
| **改** | Decisions 段加上"否决的方案"；Verification 段加上「事件日志重放重建 state」与「handler.rs 不再 import memory/llm 的 lint」这类可机器验证的判据；点名 markdown parser 与 O(n) IO 是迁移要顺手解决的债。 |
| **降权** | yaml schema 不要试图在 Phase 1 就承担"驱动 prompt"的责任；先承担 capability/config/event 索引，prompt 内容下一阶段再迁。 |
