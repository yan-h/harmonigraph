---
name: orchestrate
description: Decide what to delegate to a subagent and what to just do, and size the pieces so neither the orchestrator nor any agent drowns in context. Use when a task is big enough that fanning it out is tempting — a sweep, an audit, a multi-file change, anything where "spawn some agents" is the first instinct.
---

# Delegation costs more than doing it. Know what you are buying.

The measured names below (`Agent`, `Workflow`, fable and opus) describe the
Claude harness that produced the data. Other agents use their native
subagent controls while preserving the sizing rules, authorization boundary,
and return contract; the invocation syntax is not part of the workflow.

Measured over 434 subagent transcripts in this project:

| | median | p75 | p95 |
|---|---|---|---|
| startup context before any work | 13.6k | 18.1k | 27.1k |
| **output tokens the agent produces** | **45.6k** | 60.4k | 85.5k |
| model responses it runs for | 43 | 63 | 105 |
| report landing back here | 1.5k | 4.5k | 6.3k |

A **response** here is one API call's worth of model output — thinking, any
text, and however many `tool_use` blocks ride along. It averages 1.15 tool
calls, since parallel calls travel in a single response, so responses and tool
calls are close but not equal.

Against that, a `Read` result is 21 tokens at the median and 597 at p75; a
`Bash` result is 126. So one delegation is worth roughly a hundred inline
reads, and startup is the small half of it — the expense is that an agent
with a mandate works to its mandate whatever the size of the question.

The 30:1 squeeze (46k of work returns as 1.5k) is real, and it is the whole
point. But it is a squeeze on CONTEXT, not on tokens. Delegation buys exactly
three things:

1. **This context stays clean.** Autocompact is off here on purpose, so a
   session has to fit. Hygiene is not a saving, it is what stops a long task
   hitting the wall.
2. **Wall-clock**, when the pieces are genuinely independent.
3. **A better model or effort than this session runs.** A fable session
   delegating to opus is buying quality, not economy.

A delegation that buys none of those is a delegation that spent 46k output
tokens to avoid reading a file.

## Default to doing it here. Delegate on a trigger.

Any ONE of these is enough to delegate:

- **Volume** — answering means pulling more than ~10k tokens of material into
  this context, and most of it will not survive into the answer. Grep sweeps,
  log trawls, "which of these 40 files does X" all qualify.
- **Independence** — there are 3+ pieces that do not need each other's
  results. Below three, the startup cost is most of what you spend.
- **Tier** — the work wants a stronger model or higher effort than this
  session is running. Pass `model` on the call; effort comes from the agent
  definition's frontmatter, since the Agent tool has no effort parameter.

Any ONE of these overrides the triggers and means do it here:

- **You can name the file and the symbol.** Read it. This is the hard rule
  below.
- **The result feeds your very next tool call.** A 43-response round trip to
  unblock one decision is the wrong shape; you will sit idle for it.
- **Briefing costs more than doing.** An agent starts blind. If the task only
  makes sense given half this conversation, writing that brief is the task.

Reading files yourself is not a failure of orchestration. A task that needs
four files read and one edit made is a task to do, and the version of it that
spawns four agents is slower, more expensive, and worse.

## The hard rule

**Never delegate a lookup you could name the file for.** Everything else on
this page is judgment; this one is not, because it is the mistake that
actually recurs and it is cheap to check — if you could write the `Read` call,
write it.

## Size by question, not by file count

One agent answers one QUESTION. Startup is a fixed 13.6k whatever you ask, so
four related questions belong in one agent, not four — and an agent handed one
question per file is paying startup per file.

The opposite failure is one question with no boundary ("review the codebase").
The fix is to split the QUESTION — by subsystem, by failure mode, by reading —
never to hand it fewer files and the same open mandate.

**Count responses, not context.** Peak context is the lagging indicator: it
tracks responses at roughly 3.2k each, so it only tells you an agent was
over-scoped once it has finished and you have already paid. Measured here:

| responses | median peak context |
|---|---|
| 1–24 | 72k |
| 25–49 | 126k |
| 50–99 | 188k |
| 100+ | 290k |

Aim for an agent that finishes in 25–60 responses; the median is 43, and the
work-shaped agents here sit inside that (`Explore` 31 and
`general-purpose` 46). Past ~100 — the p95 — it has usually stopped answering
and started exploring, which is judgment rather than a measurement, since
nothing here scores an agent's answer.

`merge-auditor` runs 77 and is not mis-scoped; a question that is genuinely
wider costs more responses, and the number is a prompt to check the boundary,
not a budget to enforce.

Do not set a context ceiling instead. A subagent's peak is 142k at the median
and 250k at p90, so any round number in that range is the tail of ordinary
behaviour rather than a fault: `merge-auditor` alone has a 236k MEDIAN, because
auditing a merge range honestly needs the material, while a mis-scoped
review agent at 116k would sail under the same line.

## State the return contract in the prompt

The median report is 1.5k tokens and that is the target, not a floor. Ask for
the conclusion plus `file:line` evidence, and say that the transcript of how
it got there is not wanted. An agent that is not told this writes an essay,
and the essay lands here.

Where the answer has a shape you know in advance, say so in the prompt —
"return N findings, each with file, line, and what breaks" — and the report
comes back at a size you can predict.

## When the fan-out is the point

For genuinely parallel work over a known list, the `Workflow` tool is the
better instrument than N `Agent` calls: subagent transcripts never enter this
context at all, only each call's return value, and `schema` forces a validated
object instead of prose. It also takes `effort` per agent, which the `Agent`
tool does not.

It needs Yan's explicit opt-in — but a skill's instructions count as that
opt-in, so this section IS the opt-in when the work warrants it. Scout inline
first to find the work-list, then hand the list to the workflow; do not spawn a
workflow to discover what the list is.

## Pin the model where it must not vary

Subagents inherit this session's model unless the call overrides it. A fable
session in practice overrides to opus on most calls but not all — one review role here
ran on both, 3 spawns opus and 7 fable, decided fresh each time. Where a role must not vary, pin `model:` in its
`.claude/agents/*.md` frontmatter and stop relying on the coin flip.

## Re-deriving these numbers

They come from the subagent transcripts at
`~/.claude/projects/<proj>/<session>/subagents/agent-*.jsonl`, with the
`agentType` in the sibling `.meta.json`. Two traps sit in that format, and
both give plausible wrong answers rather than obvious ones:

- **A line is one content BLOCK, not one response.** Thinking, text and each
  `tool_use` are separate entries carrying the same `message.id` and the same
  `usage`, so counting assistant lines counts one response two to four times.
  Deduplicate on `message.id`. The tell that something is wrong is a high
  share of text-only responses and almost no parallel tool calls, neither of
  which a working agent produces.
- **`usage` is a streaming snapshot.** Earlier lines of a response carry a
  placeholder `output_tokens` of 1 or 3; only the last carries the total. Take
  the max per id. Summing lines overcounts by single digits, but reading the
  first undercounts by about 25x. A first value and a max are both immune to
  the repetition, which is why startup and peak context survive either
  mistake.
