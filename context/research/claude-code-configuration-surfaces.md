# Claude Code Configuration Surfaces

## Summary

Claude Code's configuration model has four major user-facing surfaces:

- `CLAUDE.md` memory/instructions
- `.claude/agents/*.md` subagent definitions
- `.claude/skills/*/SKILL.md` skills and custom commands
- `.claude/settings.json` and `.claude/settings.local.json` for settings, permissions, hooks, plugins, and behavior

The useful pattern for Hope is not to clone Claude's folders exactly. The useful pattern is the layered mental model:

- stable project memory/instructions
- agent declarations as markdown with YAML frontmatter
- skills as lazy-loaded task workflows with supporting files
- commands as user-invoked workflows
- settings as explicit policy and permission configuration
- hooks as lifecycle automation around prompts, tool calls, subagents, tasks, and session events

## Settings And Scopes

Claude Code uses hierarchical scopes:

- managed: organization-wide settings deployed by admins
- user: `~/.claude/`
- project: `.claude/`
- local: `.claude/settings.local.json`

Priority is managed, command-line/session, local, project, then user. Scalar settings are overridden by higher-priority scopes, while array-valued settings such as permission rules may merge.

Important scope mappings:

- settings: `~/.claude/settings.json`, `.claude/settings.json`, `.claude/settings.local.json`
- subagents: `~/.claude/agents/`, `.claude/agents/`
- MCP servers: user/global config plus project `.mcp.json`
- `CLAUDE.md`: `~/.claude/CLAUDE.md`, project `CLAUDE.md` or `.claude/CLAUDE.md`, and local `CLAUDE.local.md`

Claude's internal system prompt is not published. Custom instructions are added through `CLAUDE.md` or an append-system-prompt flag.

## Subagents

Claude subagents are Markdown files with YAML frontmatter. They can live in:

- managed settings
- the `--agents` CLI flag for a session
- project `.claude/agents/`
- user `~/.claude/agents/`
- plugin `agents/`

Project subagents are intended to be checked into a repository. User subagents are personal and cross-project. Plugin subagents have the lowest priority and have some restrictions.

Subagent files look like:

```md
---
name: code-reviewer
description: Reviews code for quality and best practices
tools: Read, Glob, Grep
model: sonnet
---

You are a code reviewer. When invoked, analyze the code and provide
specific, actionable feedback on quality, security, and best practices.
```

Only `name` and `description` are required. The body becomes the subagent's system prompt. Claude's docs say subagents receive this prompt plus basic environment details, not the full default Claude Code system prompt.

Supported frontmatter includes:

- `name`
- `description`
- `tools`
- `disallowedTools`
- `model`
- `permissionMode`
- `maxTurns`
- `skills`
- `mcpServers`
- `hooks`
- `memory`
- `background`
- `effort`
- `isolation`
- `color`
- `initialPrompt`

Important behavior:

- If `tools` is omitted, a subagent inherits available tools from the main conversation.
- `tools` is an allowlist; `disallowedTools` removes tools.
- `mcpServers` can scope MCP access to a subagent.
- `skills` preloads full skill content into the subagent at startup.
- `background: true` makes a subagent run as a background task.
- `isolation: worktree` gives the subagent a temporary git worktree.
- `memory` can enable persistent memory with user, project, or local scope.
- Plugin subagents ignore `hooks`, `mcpServers`, and `permissionMode`.
- Subagents cannot nest infinitely in the same way as main conversations; Claude's docs call out limits around built-in planning delegation.

Claude also has experimental forked subagents. A fork inherits the full conversation, model, tools, and history, unlike normal named subagents that start with a fresh isolated context. This is useful for parallel exploration but weakens input isolation.

Subagent transcripts are stored separately under the Claude project/session storage and can be resumed. The default cleanup period is 30 days.

## Skills

Claude skills are folder-based packages:

```txt
my-skill/
  SKILL.md
  scripts/
  references/
  assets/
```

Claude Code skills follow the Agent Skills open standard, with Claude-specific extensions. A skill's `SKILL.md` has YAML frontmatter plus markdown instructions. The skill directory name becomes a slash command unless a `name` is supplied.

Skill locations:

- enterprise/managed
- personal: `~/.claude/skills/<skill-name>/SKILL.md`
- project: `.claude/skills/<skill-name>/SKILL.md`
- plugin: `<plugin>/skills/<skill-name>/SKILL.md`

Claude loads skill descriptions into context for discovery, but loads the full skill body only when invoked. This is the key context-saving pattern.

Important skill frontmatter includes:

- `name`
- `description`
- `when_to_use`
- `argument-hint`
- `arguments`
- `disable-model-invocation`
- `user-invocable`
- `allowed-tools`
- `model`
- `effort`
- `context`
- `agent`
- `hooks`
- `paths`
- `shell`

Invocation controls:

- By default, users can invoke a skill with `/skill-name`, and Claude can invoke it automatically.
- `disable-model-invocation: true` makes the skill user-invoked only. Claude recommends this for side-effectful workflows such as deploys.
- `user-invocable: false` makes the skill model-invoked only.

Other important behaviors:

- `allowed-tools` pre-approves specific tools while the skill is active.
- `context: fork` runs the skill in a forked subagent context.
- `agent` selects which subagent type to use when `context: fork` is set.
- `paths` limits automatic activation to matching file patterns.
- Skills can contain dynamic shell injections using `!` command syntax, where command output is inserted into skill content before the model sees it.
- Supporting files let large references stay out of context until needed.

Claude recommends keeping `SKILL.md` concise because once loaded, its content stays in context across turns.

## Commands

Claude has built-in slash commands and user-defined commands.

Current Claude docs say custom commands have effectively merged into skills:

- `.claude/commands/deploy.md` still works.
- `.claude/skills/deploy/SKILL.md` also creates `/deploy`.
- If a skill and command share a name, the skill takes precedence.
- Skills are now recommended because they support supporting files and richer frontmatter.

Claude commands include product/runtime controls such as:

- `/agents`
- `/tasks`
- `/background`
- `/resume`
- `/branch`
- `/plan`
- `/compact`
- `/context`
- `/mcp`
- `/permissions`
- `/skills`
- `/loop`
- `/batch`
- `/simplify`
- `/review`
- `/security-review`
- `/schedule`
- `/remote-control`

Several commands are implemented as bundled skills rather than fixed CLI behavior. Claude's command reference marks these as skills.

MCP servers can also expose prompts that appear as commands using `/mcp__<server>__<prompt>`.

## Hooks

Claude hooks are user-defined shell commands, HTTP endpoints, or LLM prompts that run at lifecycle points. They can inspect JSON event context and optionally return decisions.

Events include:

- `SessionStart`
- `UserPromptSubmit`
- `UserPromptExpansion`
- `PreToolUse`
- `PermissionRequest`
- `PostToolUse`
- `PostToolUseFailure`
- `SubagentStart`
- `SubagentStop`
- `TaskCreated`
- `TaskCompleted`
- `Stop`
- `ConfigChange`
- `FileChanged`
- `WorktreeCreate`
- `PreCompact`
- `PostCompact`
- `SessionEnd`

The pattern is useful for Hope, but the implementation should be server/client aware. In Hope, hooks should be policy-checked and should not be allowed to smuggle hidden prompts, secrets, or unrestricted local execution into the cloud harness.

## MCP

Claude Code supports MCP as the integration layer for external tools, databases, APIs, and event channels.

Supported transports include:

- remote HTTP
- deprecated SSE
- local stdio

MCP servers can be installed at local, project, user, plugin, or managed scopes. Claude supports dynamic tool updates, reconnection, MCP resources, MCP prompts as commands, and managed allowlists/denylists.

For Hope, the important lesson is not "send every MCP tool to the model." The useful pattern is:

1. discover local/project MCP manifests
2. cache them server-side by hash
3. search/rank tools server-side
4. expose a small selected toolset per run or step
5. execute local MCP calls through the local client

## Implications For Hope

Recommended Hope shape:

```txt
.hope/
  hope.md
  settings.json
  agents/
    developer.md
    reviewer.md
    researcher.md
  skills/
    fix-failing-test/
      SKILL.md
  commands/
    implement.md
    research.md
```

Agent files should be Markdown with YAML frontmatter:

```md
---
type: agent
name: developer
description: Implements scoped code changes and verifies them.
tools:
  allow:
    - file.read
    - file.search
    - file.patch
    - shell.run
skills:
  default:
    - fix-failing-test
effort: high
background: false
---

Implement changes carefully:
- Inspect before editing.
- Keep changes scoped.
- Preserve user work.
- Run focused verification.
- Report changed files and tests run.
```

Hope should copy:

- markdown plus frontmatter for agents
- folder-based skills with `SKILL.md`
- lazy skill loading
- user-invoked and model-invoked distinction
- supporting files for skills
- command-as-skill convergence
- explicit tool allowlists/denylists
- background runs and resumable tasks
- worktree/sandbox isolation concept
- lifecycle hooks as a later extension
- MCP as a first-class tool surface

Hope should improve or diverge:

- Keep the proprietary harness cloud-side instead of shipping the full brain.
- Make runs the primary API object.
- Make the event log the durable trace.
- Treat local client as a tool executor, not the orchestrator.
- Cache project config, tools, and skills by hash in the cloud.
- Use server-side skill/tool ranking before model context injection.
- Avoid dumping all available tools into every call.
- Keep customer data training policy explicit and trust-first.
- Keep champion/challenger promotion separate from customer runtime.

## Sources

- Claude Code subagents: https://code.claude.com/docs/en/sub-agents
- Claude Code skills: https://code.claude.com/docs/en/skills
- Claude Code commands: https://code.claude.com/docs/en/commands
- Claude Code settings: https://code.claude.com/docs/en/settings
- Claude Code hooks: https://code.claude.com/docs/en/hooks
- Claude Code MCP: https://code.claude.com/docs/en/mcp
- Agent Skills overview: https://agentskills.io/home
