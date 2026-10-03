# AGENTS.md - MyAI

## App

`MyAI` provides the `ai` CLI tool. All-in-one AI tool.

- `ai memory`: permanent memory for AI agents

## Working

- Delegate implementation work to dedicated subagents. Follow logical sequencing when there are dependencies between tasks. Multiple agents can be spawned or work that can be done parallely.
- AI are provided with a workspace. This is the primary home for any AI assistant. `ai`, `ai/research`, `ai/comments`, etc.
- Humans are provided with a workspace. This is their primary home. Do NOT edit anything in this directory. `my`, `my/notes`, etc.
- The main tree is for AI-assisted files. These are worked on by BOTH AI and humans. And should have minimal changes.
- Glossary is given in `my/glossary.md`. Maintain `ai/glossary.md` yourself.
- Backlog is given in `ai/backlog.md`.

## Writing
- Prefer terse prose
