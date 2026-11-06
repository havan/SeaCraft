---
name: seacraft-contributing
description: Use when someone wants to contribute to SeaCraft, from choosing an issue through sending a pull request, or when cutting a version.
---

# Contribute to SeaCraft

Read [CONTRIBUTING.md](../../CONTRIBUTING.md) and [AGENTS.md](../../AGENTS.md) first.

## Every pull request

- **Look at what is open first**, so two people do not do the same work.
- **Claim the work.** Comment on the issue or open a draft PR. If someone has claimed it, talk to them first.
- **One topic per PR**, with a title and first sentence that make sense to someone who has seen nothing else.
  Say which part of the app changes and what a user now sees or can do.
- **Run `cargo xtask ci`** before each commit.
- **Show visible changes** with real screenshots captured from the app: before and after, in the PR
  description, not committed to the repository.
- **A change to a number** says which numbers moved, by how much and why, and points to the oracle.
- **Write in English**: code, comments, commit messages and docs.

## Working as an agent

- Run the app and capture evidence yourself, rather than asking a person. `seacraft render` draws images
  without a window, and `seacraft-gui --control HOST:PORT` drives the real GUI.
- Never report a test count or a pass you did not read in the output.
- Do not commit session notes, or name your model or tools in the code. A commit explains the change.

## Commits and versions

Commit subjects follow `type(scope): summary`, such as `fix(core): ...`. History stays linear.

* Semver, one version shared through `[workspace.package] version`. `0.2.N` for a fix-only change,
  `0.3.0-beta.N` per completed feature until the next readiness review, whose criterion is: open, edit, measure,
  load, report, save and reopen with no known blocking defect.
* The version must be identical in `Cargo.toml`, `Cargo.lock`, the window title and `seacraft --version`.
* The maintainer may commit directly to `main`. Everyone else sends a pull request.

## Reference

`docs/architecture.md` describes the design, and `ROADMAP.md` what comes next. Update `ROADMAP.md` when a
milestone task lands, and `CHANGELOG.md` for anything a user would notice.
