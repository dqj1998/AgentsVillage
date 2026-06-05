# AgentsVillage

An agent platform for running AI agents across message channels, built in Rust.
Discord is the first supported channel adapter; each Discord channel or thread
gets its own agent with persistent memory, configurable LLM backend, and a
schema-backed identity. Future adapters can bring other channels, such as
Telegram, into the same request pipeline.

## Quick Start

### 1. Prerequisites

- Rust (stable, 1.75+)
- A Discord bot token ([Discord Developer Portal](https://discord.com/developers/applications))
- An LLM API key (OpenRouter) or a local Ollama instance

### 2. Configure

Run the setup wizard on first launch:

```sh
cargo run
```

This creates `config.toml`. You can also copy and edit manually:

```toml
[core]
context_window = 20        # number of recent messages to include in LLM context

[llm]
provider = "openrouter"    # or "ollama"
base_url = "https://openrouter.ai/api/v1"
model = "openai/gpt-4o-mini"
timeout_secs = 30

[discord]
application_id = "YOUR_APP_ID"
guild_id = "YOUR_GUILD_ID"
channel_id = "YOUR_CHANNEL_ID"
```

### 3. Set environment variables

Create a `.env` file:

```
DISCORD_TOKEN=your-discord-bot-token
LLM_API_KEY=your-openrouter-api-key   # omit for Ollama
```

### 4. Run

```sh
cargo run
```

## Request Flow

AgentsVillage routes channel traffic through a single platform-agnostic pipeline:

`Channel Adapter → AppRequest → Intent → AppService/ExecutionEngine → AppResponse`

Chat execution writes session transcripts, event logs, and schema-backed agent state while preserving the existing role and memory files under each workspace.

System-level schema lives in [schemas/system.yaml](schemas/system.yaml). Workspace files
hold one agent instance's manifest, channel binding, policy overrides, runtime
state, transcripts, and event log.
Executable intents are validated against the system schema before session writes,
event writes, or LLM calls.

## Agent Workspace Layout

Each agent stores its data under `workspace/{agent_id}/`:

```
workspace/{agent_id}/
  manifest.yaml            — agent instance manifest and system schema refs
  channel-binding.yaml     — external channel binding for this agent instance
  role.md                  — system prompt (edit this to change agent personality)
  memory.md                — long-term memory summaries
  sessions/
    YYYY-MM-DD.md          — daily conversation transcripts
  schema/
    agent.yaml             — legacy instance identity compatibility file
    capabilities.yaml      — instance policy overrides for intent, memory, audit, and safety
    state.yaml             — runtime state compatibility file
  events/
    events.jsonl           — append-only event log
```

## How to Add a New Agent

1. Create a workspace directory: `workspace/discord-{guild_id}-{channel_id}/`
2. Write a `role.md` with the agent's system prompt
3. (Optional) Add `schema/capabilities.yaml` to configure executable policies
4. (Optional) Add `config.toml` in the workspace to override LLM model or context window
5. Point the Discord channel adapter at the channel — the agent is created automatically on first message

Example `role.md`:
```markdown
You are a helpful assistant specializing in Rust programming.
Answer questions clearly and provide working code examples.
```

Example per-agent `config.toml`:
```toml
[llm]
model = "anthropic/claude-3-haiku"

[core]
context_window = 10
```

Example `schema/capabilities.yaml`:
```yaml
intent_policy:
  allow_chat: true
  allow_reset_session: true
  command_mode: reject_unknown
memory_policy:
  context_window: 20
  summarize_old_messages: true
  write_summary_to_memory: true
safety_policy:
  deny_disabled_intents: true
  require_explicit_user_command_for_reset: true
```

The repo-level system schema describes architecture and audit contracts:

```
schemas/
  system.yaml
```

## Slash Commands

| Command | Description |
|---------|-------------|
| `/new` | Clear today's session and start fresh (long-term memory is preserved) |

## Architecture

See [docs/architecture.md](docs/architecture.md) for the full architecture reference, including module layout, pipeline diagrams, intent lifecycle, event log schema, and boundary rules.
