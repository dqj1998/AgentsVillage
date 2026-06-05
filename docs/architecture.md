# AgentsVillage Architecture

## Overview

AgentsVillage is an agent platform that routes channel traffic through a layered
`Adapter → AppRequest → Intent → Store/Executor → AppResponse` pipeline. Discord
is the first supported channel adapter; future adapters can map other channels,
such as Telegram, into the same platform-agnostic request and response flow.

## Module Layout

```
src/
  main.rs              — startup assembly, wires AppService + DiscordHandler
  config.rs            — GlobalConfig, CoreConfig (context_window), merge helpers
  llm.rs               — LlmClient (HTTP wrapper for OpenRouter/Ollama)
  error.rs             — AppError enum
  agent/
    model.rs           — Agent (id, display_name, workspace_path, role, config, capabilities, state)
    manager.rs         — AgentManager (load/create agents, lazy SchemaStore init)
    memory.rs          — MemoryManager (session/memory markdown read/write)
    schema_store.rs    — SchemaStore (agent.yaml, capabilities.yaml, state.yaml)
    event_log.rs       — EventLog (append-only JSONL: ChatStarted/Completed/Failed/ResetSession/MemorySummarized)
  app/
    mod.rs             — AppRequest, RequestPayload, Intent, AppResponse
    compiler.rs        — IntentCompiler (Message→Chat, /new→ResetSession, Command→Command)
    executor.rs        — ExecutionEngine, ChatExecutor (writes EventLog, uses SchemaRenderer)
    renderer.rs        — SchemaRenderer (assembles system prompt from role + memory + capabilities)
    schema.rs          — AgentSchema, AgentCapabilities, policy types, AgentState
    service.rs         — AppService::handle() — the platform-agnostic entry point
  discord/
    adapter.rs         — Discord platform helpers (build_agent_id, split_message, get_thread_id, etc.)
    handler.rs         — DiscordHandler (Serenity EventHandler, routes to AppService)
    gateway.rs         — Discord client builder
    setup.rs           — Init wizard re-export
schemas/
  system.yaml          — platform pipeline, intents, executors, audit, memory, and channel contracts
```

## Request Pipeline

```
Channel Event (Discord today)
  → DiscordHandler::message()
  → AppRequest { agent_id, platform_user, timestamp, payload }
  → AppService::handle()
    → IntentCompiler::compile()  → Intent
    → AgentManager::get_or_create_agent()  → Agent (schema-backed)
    → ExecutionEngine::execute()
      → SystemSchemaCatalog::validate_intent_execution()
      → ChatExecutor::execute()
        → MemoryManager::append_session()
        → SchemaRenderer::render_system_prompt()
        → build_llm_messages_with_summary()
        → LlmClient::chat()
        → MemoryManager::append_session()
        → EventLog::append(ChatCompleted)
  → AppResponse::Text(s)
  → adapter::split_message()
  → Channel send
```

## Agent Workspace Layout

```
workspace/{agent_id}/
  manifest.yaml              — runtime agent instance manifest and system schema refs
  channel-binding.yaml       — external channel binding for this agent instance
  role.md                    — system prompt source
  memory.md                  — long-term memory summaries
  sessions/
    YYYY-MM-DD.md            — daily session transcript
  schema/
    agent.yaml               — legacy instance identity compatibility file
    capabilities.yaml        — instance policy overrides plus legacy flags
    state.yaml               — runtime state compatibility file
  events/
    events.jsonl             — append-only event log
```

`schemas/system.yaml` is the repo-level system schema. It expresses platform architecture,
intent execution, executor side-effect boundaries, audit requirements, memory
effects, and channel contracts. `workspace/{agent_id}/` contains instance data:
which agent exists, which channel it is bound to, its local policy overrides,
runtime state, transcripts, and observed events.

## Intent Lifecycle

1. Channel adapter (Discord today) receives event
2. `DiscordHandler` constructs `AppRequest`
3. `IntentCompiler` maps `RequestPayload` → `Intent` enum
4. `ExecutionEngine` dispatches to appropriate executor
5. Executor reads agent state, calls LLM if needed, writes events
6. `AppResponse` returned to handler
7. Handler maps response to channel action (send message, ephemeral, etc.)

## System Schema And Instance Policy

System schema lives in the repository, not inside a runtime workspace:

```text
schemas/system.yaml      = how AgentsVillage is architected
workspace/{agent_id}/    = one agent instance and its channel binding
workspace/{agent_id}/events/ = what actually happened
```

`schemas/system.yaml` describes intent execution at step granularity. For
example, `chat` declares the session write, prompt render, LLM call, assistant
message write, failure behavior, and required audit events. The same file also
declares audit event names, required fields, executor side-effect boundaries,
memory effects, and Discord delivery/routing constraints.

At runtime, `ExecutionEngine` loads the embedded system schema catalog and
validates executable intents before side effects. The validator checks that the
intent exists, the declared executor matches, each intent step uses an effect
allowed by the executor section, referenced audit events exist in the audit
section, and the agent instance policy does not suppress audit
events required by the intent schema. If validation fails, execution stops before
session writes, event writes, or LLM calls.

Workspace files connect an instance to the system schema:

```yaml
# workspace/{agent_id}/manifest.yaml
manifest_version: 1
agent_id: discord-...
display_name: general
schema_refs:
  system: schemas/system.yaml
```

```yaml
# workspace/{agent_id}/channel-binding.yaml
binding_version: 1
agent_id: discord-...
channel:
  kind: discord
  external_id: discord-...
  fields:
    guild_id: "..."
    channel_id: "..."
```

The legacy `workspace/{agent_id}/schema/` directory is still read for backward
compatibility while the runtime moves toward manifest, binding, policy, and
state as separate instance concerns.

## Instance Policy Overrides

`schema/capabilities.yaml` currently stores per-agent policy overrides. The
engine consults it before applying side effects:

```yaml
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
```

`ExecutionEngine` currently enforces system schema validation, chat/reset gates,
memory summarization policy, and audit event emission. Older flat flags (`chat`,
`reset_session`, `summarize`, `commands`) are still accepted for existing
workspaces.

## Intent Types

| Intent | Trigger | Executor behavior |
|--------|---------|-------------------|
| `Chat { user_text, author }` | Any message | Append to session, build prompt, call LLM, append response, write ChatStarted/Completed events |
| `ResetSession` | `/new` command | Clear today's session, write ResetSession event |
| `Command { name, args }` | Other slash commands | Return ephemeral "Unknown command" |
| `Clarify { question }` | (future) | Return question as text |

## Event Log Schema

Events are stored as JSONL in `workspace/{agent_id}/events/events.jsonl`.

```json
{"type":"chat_started","timestamp":"2024-01-01 00:00:00 UTC","author":"alice","user_text_len":42}
{"type":"chat_completed","timestamp":"2024-01-01 00:00:01 UTC","response_len":128}
{"type":"reset_session","timestamp":"2024-01-01 00:01:00 UTC","reason":"user_command"}
{"type":"memory_summarized","timestamp":"2024-01-01 00:02:00 UTC","messages_summarized":15}
```

## Configuration

`config.toml`:
```toml
[core]
context_window = 20        # messages to keep in LLM context

[llm]
provider = "openrouter"    # or "ollama"
base_url = "https://openrouter.ai/api/v1"
model = "openai/gpt-4o-mini"
timeout_secs = 30

[discord]
application_id = "..."
guild_id = "..."
channel_id = "..."
```

Per-agent override in `workspace/{agent_id}/config.toml`:
```toml
[llm]
model = "anthropic/claude-3-haiku"

[core]
context_window = 10
```

## Architecture Boundaries

`src/discord/handler.rs` MUST NOT directly call:
- `MemoryManager` methods
- `LlmClient::chat()`

Those actions are handled through `AppService` and `ExecutionEngine`.

Per-agent config can still override `[core] context_window` and LLM settings.
