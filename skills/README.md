# Agent skills

Skills for AI agents that design, fair and check hulls in SeaCraft through its MCP server
(`seacraft mcp`, see the MCP section of the [main README](../README.md)). Each folder is one skill in the
[Agent Skills](https://agentskills.io) format: a `SKILL.md` with a name and a description that tell the
agent when to load it.

| Skill | Loads when the agent |
|---|---|
| [`seacraft-hull-design`](seacraft-hull-design/SKILL.md) | loads a hull and must hit targets: displacement, draft, LCB, Cp, main dimensions, trim for least resistance |
| [`seacraft-fairing`](seacraft-fairing/SKILL.md) | smooths a hull: zebra or curvature renders, linesplan combs, small control-point moves |
| [`seacraft-stability-check`](seacraft-stability-check/SKILL.md) | reports hydrostatics, KN/GZ cross curves, IMO A.749 results, or Holtrop / Savitsky resistance |

They teach the agent to:
- read the hull and a baseline measurement before touching anything
- change one thing at a time and re-measure against numbers it computed itself
- look at the hull with `render_png` instead of guessing from coordinates
- report KB, KM and KG above the keel, trim as a length, and state the loading condition

They also list the traps: the default draft, `free_trim` versus fixed trim, `mainframe_x` moving Cp, `transform_hull` scaling
the whole model, and a half model that needs a transom face.

## Install

Claude Code, for all projects:

```sh
cp -r skills/seacraft-* ~/.claude/skills/
```

One project only: copy the folders into that project's `.claude/skills/` instead. Other agents that read the
Agent Skills format: copy them into the skills directory their documentation names.

Register the server (headless, edits stay in the server until `save_hull`):

```sh
claude mcp add seacraft -- seacraft mcp --open hull.fef
```

To drive a live modeller, start the GUI with a control port and connect to it:

```sh
seacraft-gui --control 127.0.0.1:7878 --open hull.fef
claude mcp add seacraft -- seacraft mcp --connect 127.0.0.1:7878
```

## Maintaining

- `skills_name_only_real_tools` in `crates/seacraft-cli/src/mcp.rs` runs in CI. It reads every `SKILL.md`, checks that
  each `` `tool_name {…}` `` call names a tool in `tool_definitions()`, and checks the frontmatter (`name` equals the
  folder, `description` starts "Use when").
- Numbers quoted in the skills come from the demo hull (`examples/demo-hull.fef`, T = 0.62 m). If the reference
  numbers in `AGENTS.md` move, update the skills in the same change.
- Write tool calls as `` `tool_name {"arg": value}` `` so the test sees them; keep other backticked code from
  starting with `name {`.
- Each skill ends with the SeaCraft version it was written against; bump it when its tools change.
