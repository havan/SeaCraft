# Contributing to SeaCraft

SeaCraft is an open-source hull modeller and naval-architecture tool in Rust. Help is welcome, from people and
from their AI agents, whether that is a one-line fix, a comparison with published data, or a whole feature. This
is mostly one person's project, so reviews may take a while. The rules every change follows are in
[AGENTS.md](AGENTS.md).

## Get it running

You need [rustup](https://rustup.rs), which installs the Rust version in `rust-toolchain.toml`. On Linux, the GUI also needs the usual windowing libraries (see
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) for the Ubuntu package list).

```sh
git clone https://github.com/havan/SeaCraft && cd SeaCraft
cargo run -p seacraft-gui -- --open examples/demo-hull.fef   # the desktop app
cargo run --bin seacraft -- hydro examples/demo-hull.fef --kg 2.2
cargo xtask ci                                               # the gate
```

The design is in [docs/architecture.md](docs/architecture.md).

## Where help is wanted most

1. **Checking the numbers.** Most results have never been compared with reality. If you can compare SeaCraft with
   a published hull, a tank test or approved software, you can add it as an oracle in
   [docs/oracles.md](docs/oracles.md) without touching solver code. A disagreement is as useful as a match: open an
   issue.
2. **Viewport performance** and the rest of [ROADMAP.md](ROADMAP.md).
3. **Bugs**, especially wrong numbers with a small reproduction.

## Rules that aren't negotiable

- **Numbers need a source.** A test's reference number says where it came from. "Published" means that you read the
  published source.
- **Be honest about results.** Don't describe a result as verified or compliant without saying against what. See
  [DISCLAIMER.md](DISCLAIMER.md).
- **Licences.** SeaCraft is `GPL-3.0-or-later`. Dependencies and ported code must be compatible, and ported code is
  listed in [ATTRIBUTION.md](ATTRIBUTION.md). Contributions are under the same licence.
- **No local notes or session logs** in the repository.

## Sending a change

1. Look at open issues and PRs first. Comment on the issue, or open a draft PR, to claim the work.
2. Keep to one topic per PR, and write the title for someone who has seen nothing else.
3. Run `cargo xtask ci`.
4. For anything visible, put real before and after screenshots in the PR description. `seacraft render` and
   `seacraft-gui --control HOST:PORT` make this easy for agents.
5. If a number moves, say which, by how much, and why.

Agents: `AGENTS.md` routes you to the skill for your task. Whoever opens the PR is responsible for what is in it.

## Licence

By contributing you agree that your contribution is licensed under `GPL-3.0-or-later`.
