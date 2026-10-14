<div align="center">

<h1>
  <img src="assets/logo-dark.gif#gh-dark-mode-only" alt="CometixCode" height="40" align="top"><img src="assets/logo-light.gif#gh-light-mode-only" alt="CometixCode" height="40" align="top">
  CometixCode
</h1>

English | [简体中文](README_CN.md)

![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust&logoColor=white)
![License](https://img.shields.io/badge/License-AGPL--3.0-blue)

</div>

A terminal-based AI coding assistant, written in Rust as a 1:1 reimplementation
of Anthropic's Claude Code.

> **Unofficial project, not affiliated with Anthropic.**
> This is an independent reimplementation built by reading the published
> Claude Code CLI. "Claude" and "Claude Code" are trademarks of Anthropic, PBC.
> Nothing here is endorsed by or supported by Anthropic.

## What this is

Claude Code's terminal UI is TypeScript on React + Ink. CometixCode reproduces
it in Rust on iocraft, a React-style retained-mode TUI framework that is the
structural counterpart of Ink — components, hooks, declarative elements. The
port follows the original file by file rather than reinventing the
architecture: a TypeScript component maps to a Rust component, a hook to a
hook, and deliberate deviations are recorded in the source where they happen.

It builds against [CometixTUI], a fork of iocraft carrying the primitives this
port needs (row-level diffing, event propagation, SIGCONT self-healing, IME
cursor, bracketed paste, grid layout).

It is a work in progress. Large parts of the interactive loop, tool execution,
permissions, MCP and slash commands are implemented; other areas are partial.

## Sponsors

<p align="center">
  <a href="https://www.infistar.cc/register?aff=HALLEY#gh-dark-mode-only"><img src="assets/sponsor-infistar-en-dark.webp#gh-dark-mode-only" alt="Infistar.cc Sponsor"></a><a href="https://www.infistar.cc/register?aff=HALLEY#gh-light-mode-only"><img src="assets/sponsor-infistar-en-light.webp#gh-light-mode-only" alt="Infistar.cc Sponsor"></a>
</p>

<p align="center">
  <strong>CometixCode × Infistar.cc | High-Availability AI API</strong><br>
  Stable routing | 10% rates · Transparent · Failover<br>
  Top models | ChatGPT · Claude · Gemini · Kimi · GLM · DeepSeek<br>
  Multimodal | Text · Image · Video · Embedding / Rerank<br>
  CometixCode users: bonus credits via the <a href="https://www.infistar.cc/register?aff=HALLEY">exclusive link</a>.
</p>

## Building

### Prerequisites

**Rust 1.88+** (edition 2024). Nothing else — no system libraries, no
`pkg-config`.

### Build and run

```sh
cargo build --release
cargo run --release
```

The binary is `cometix`.

### Cross-compiling for Windows

```sh
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

A mingw-w64 toolchain has to be on `PATH`: four dependencies compile C or
assembly (`onig_sys`, `ring`, `tree-sitter`, `tree-sitter-bash`). They link
statically, so the resulting `.exe` needs no DLLs alongside it.

## ripgrep

File search shells out to `rg`. Claude Code ships a packaged ripgrep inside its
npm package and executes that path; CometixCode keeps the same shape — the
binary is an external asset, never compiled into the executable.

Resolution order:

1. `vendor/ripgrep/<arch>-<os>/rg` next to the executable (release archives)
2. `vendor/ripgrep/<arch>-<os>/rg` in the source tree (development)
3. the host's `rg` on `PATH`

So either drop a pinned ripgrep under `vendor/ripgrep/`, or just have `rg`
installed. `/doctor` reports which one is in use. Setting
`USE_BUILTIN_RIPGREP=0` forces the host binary.

## Testing

```sh
just test          # full suite through cargo-nextest — the gate
just t <pattern>   # substring filter
just check         # type/borrow check only, links nothing
```

`cargo test` is **not** a valid gate for this crate. It runs the whole suite as
threads in one process, and this codebase has process-wide state (env vars,
`OnceLock` caches) that leaks between tests as a result. `just test` runs under
`cargo-nextest`, which gives each test its own process, and pins the host state
a test would otherwise inherit. The justfile header explains the measurements
behind that.

## Community

<a href="https://qm.qq.com/q/nmbS5eUUP8" target="_blank"><img src="https://img.shields.io/badge/QQ%20群-1045122926-EB1923?logo=tencentqq&logoColor=white" alt="QQ Group" /></a>
<a href="https://t.me/CometixSpace" target="_blank"><img alt="telegram" src="https://img.shields.io/badge/chat-telegram-blueviolet?style=flat&logo=Telegram"></a>
[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-Community-blue)](https://linux.do/t/topic/2927016)

## Acknowledgements

- **[ClaudeCodeRev]** — the analysis work this port reads from. Every behaviour
  here was derived from studying Claude Code through it.
- **[CometixTUI]** — the TUI framework this is built on, an iocraft fork
  carrying the Ink parity primitives the port needs.
- **[marked-rs]** — Rust port of `marked`, which CC uses to parse Markdown.
- **[anthropic-sdk-rs]** — Rust port of `@anthropic-ai/sdk`, the API client.
- **[nucleo]** by the Helix editor project — the fuzzy matcher behind file,
  command and agent completion.

## License

[AGPL-3.0-only](LICENSE).

[CometixTUI]: https://github.com/Haleclipse/CometixTUI
[ClaudeCodeRev]: https://github.com/Haleclipse/ClaudeCodeRev
[marked-rs]: https://github.com/Haleclipse/marked-rs
[anthropic-sdk-rs]: https://github.com/Haleclipse/anthropic-sdk-rs
[nucleo]: https://github.com/helix-editor/nucleo
