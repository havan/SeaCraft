# Security policy

SeaCraft is young; only `main` and the latest release get fixes.

Please **don't open a public issue** for a security problem. Report it privately through the repository's
[Security tab](https://github.com/havan/SeaCraft/security/advisories/new), with what you found, how to reproduce it,
the version and platform, and the impact you expect.

Reports are especially useful for:

- the control channel (`seacraft-gui --control`): token handling, the read and write roots, anything that reaches it
  from outside the machine;
- the MCP server (`seacraft mcp`): tool arguments that escape the allowed paths;
- opening untrusted `.fef` or `.seacraft.json` files: crashes, runaway memory or CPU, path tricks.

The control channel is meant for loopback use. Exposing it to a network is not supported.

Wrong results are not security problems, so please report them as ordinary issues. See
[DISCLAIMER.md](DISCLAIMER.md).
