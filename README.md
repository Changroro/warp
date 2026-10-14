# Warp, personal fork

**English** · [한국어](README.ko.md)

A personal fork of [warpdotdev/warp](https://github.com/warpdotdev/warp) that brings the SSH workflow up to Termius level. Everything upstream still works; the features below are added on top. Upstream's README follows [further down](#about).

## SSH manager

**→ Hosts live in your real `~/.ssh/config`.** Hosts you add or rename in Warp work with `ssh <alias>` from any terminal, and hand-written hosts show up in the panel. Unknown directives, comments, and `Include`/`Match` blocks are kept as-is, and every write leaves the previous file at `~/.ssh/config.warp.bak`.

**→ Hosts panel.** Pinned hosts, tag groups, filter chips, last-connected time, and open connection counts. Select several hosts to open them at once.

**→ Launcher (`Cmd/Ctrl+Shift+H`).** Search saved hosts or type `user@host:port` to quick-connect, optionally saving it. The new-tab menu has an SSH section too.

**→ Mirror a host's Warp tabs.** Hover a host and press the layout button to reopen the tabs of the WarpOss running on that server, with the same titles, colors, splits, and working directories. Each pane SSHes into the host and `cd`s to its directory. The remote session DB is only read (`sqlite3 -readonly`), and closing these tabs leaves the server's own sessions untouched.

**→ Remote file browsing.** Open the remote host's files in Warp's file tree from an SSH session.

**→ Image paste over SSH.** Pasting a clipboard image in an SSH tab uploads it to a random `/tmp` path on the host (mode `0600`) and types that path, so Claude Code or Codex running remotely can attach it.

**→ Auto-reconnect.** When the connection drops, the session bar shows why and reconnects after a countdown, up to 5 attempts, never into a non-empty command line.

**→ Saved passwords without typing them into the terminal.** Passwords sit in the macOS Keychain or Linux secure storage. On connect, `ssh` runs WarpOss as its `SSH_ASKPASS` helper, which answers **only the first** password prompt for the host's own `user@host`. Host key checks, jump hosts, one-time codes, and retries are asked on the terminal as usual. Needs OpenSSH 8.4+.

## Session archive

Closing a tab in the vertical tabs panel keeps a record of where it was working: title, directory, git branch, the CLI agent that drove it, and when it was last used. Click an entry to reopen it in that directory. Entries can be dragged into order and are covered by tab search.

## Install

Grab a `fork-v*` build from [Releases](https://github.com/Changroro/warp/releases).

- Linux x86_64: `.deb` or `.AppImage`
- macOS Apple Silicon: `.dmg` (ad-hoc signed; if the first launch is blocked, run `xattr -dr com.apple.quarantine /Applications/WarpOss.app`)

> [!IMPORTANT]
> Remote file browsing and tab mirroring need **Settings → Warpify → Warpify SSH Sessions** turned on. Tab mirroring also needs `sqlite3` and WarpOss on the remote host.

## Branches

`master` mirrors upstream, features are built on `feat/*` and merged into `dev`, and `dev` is the build we run. Syncing and releases are described in [FORK.md](FORK.md).

<!-- Screenshots: hosts panel, launcher, tab mirroring -->

---

<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>
&nbsp;
<p align="center">
  <a href="https://warp.dev/factories"><img height="20" alt="Built with Warp" src="https://raw.githubusercontent.com/warpdotdev/brand-assets/main/Github/Built-With-Warp-Export@2x.png" /></a>
</p>

<p align="center">
  <a href="https://www.warp.dev">Website</a>
  ·
  <a href="https://www.warp.dev/code">Code</a>
  ·
  <a href="https://www.warp.dev/agents">Agents</a>
  ·
  <a href="https://www.warp.dev/terminal">Terminal</a>
  ·
  <a href="https://www.warp.dev/drive">Drive</a>
  ·
  <a href="https://docs.warp.dev">Docs</a>
  ·
  <a href="https://www.warp.dev/blog/how-warp-works">How Warp Works</a>
</p>

> [!NOTE]
> OpenAI is the founding sponsor of the new, open-source Warp repository, and the new agentic management workflows are powered by GPT models.

<h1></h1>

## About

[Warp](https://www.warp.dev) is an agentic development environment, born out of the terminal. Use Warp's built-in coding agent, or bring your own CLI agent (Claude Code, Codex, Gemini CLI, and others).

## Installation

You can [download Warp](https://www.warp.dev/download) and [read our docs](https://docs.warp.dev/) for platform-specific instructions.

## Warp Contributions Overview Dashboard

Explore [build.warp.dev](https://build.warp.dev) to:
- Watch thousands of [Warp Factory](warp.dev/factories) agents triage issues, write specs, implement changes, and review PRs
- View top contributors and in-flight features
- Track your own issues with GitHub sign-in
- Click into active agent sessions in a web-compiled Warp terminal

## Automate development with Warp Factories

This repository is driven by [Warp Factories](https://warp.dev/factories): open, flexible infrastructure for teams to build cloud software factories of their own.

Warp Factories are defined in code and easy to deploy on any model or harness, with evals, benchmarks, and self-improvement built in. [Get early access](https://warp.dev/factories) or [book a demo](https://warp.dev/get-started) for a tailored walk-through of the platform. Eligible teams can receive up to $10,000 in free factory usage.

## Licensing

Warp's UI framework (the `warpui_core` and `warpui` crates) are licensed under the [MIT license](LICENSE-MIT).

The rest of the code in this repository is licensed under the [AGPL v3](LICENSE-AGPL).

## Open Source & Contributing

Warp's client codebase is open source and lives in this repository. We welcome community contributions and have designed a lightweight workflow to help new contributors get started. For the full contribution flow, read our [CONTRIBUTING.md](CONTRIBUTING.md) guide.

> [!TIP]
> **Chat with contributors and the Warp team** in the [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB) Slack channel — a good place for ad-hoc questions, design discussion, and pairing with maintainers. New here? [Join the Warp Slack community](https://go.warp.dev/join-preview) first, then jump into `#oss-contributors`.

### Issue to PR

Before filing, [search existing issues](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc) for your bug or feature request. If nothing exists, [file an issue](https://github.com/warpdotdev/warp/issues/new/choose) using our templates. Security vulnerabilities should be reported privately as described in [CONTRIBUTING.md](CONTRIBUTING.md#reporting-security-issues).

Once filed, a Warp maintainer reviews the issue and may apply a readiness label: [`ready-to-spec`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-spec) signals the design is open for contributors to spec out, and [`ready-to-implement`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-implement) signals the design is settled and code PRs are welcome. Anyone can pick up a labeled issue — mention **@oss-maintainers** on an issue if you'd like it considered for a readiness label.

### Building the Repo Locally

To build and run Warp from source:

```bash
./script/bootstrap   # platform-specific setup
./script/run         # build and run Warp
./script/presubmit   # fmt, clippy, and tests
```

See [AGENTS.md](AGENTS.md) for the full engineering guide, including coding style, testing, and platform-specific notes.

## Joining the Team

Interested in joining the team? See our [open roles](https://www.warp.dev/careers).

## Support and Questions

1. See our [docs](https://docs.warp.dev/) for a comprehensive guide to Warp's features.
2. Join our [Slack Community](https://go.warp.dev/join-preview) to connect with other users and get help from the Warp team — contributors hang out in [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB).
3. Try our [Preview build](https://www.warp.dev/download-preview) to test the latest experimental features.
4. Mention **@oss-maintainers** on any issue to escalate to the team — for example, if you encounter problems with the automated agents.

## Code of Conduct

We ask everyone to be respectful and empathetic. Warp follows the [Code of Conduct](CODE_OF_CONDUCT.md). To report violations, email warp-coc at warp.dev.

## Open Source Dependencies

We'd like to call out a few of the [open source dependencies](https://docs.warp.dev/help/licenses) that have helped Warp to get off the ground:

- [Tokio](https://github.com/tokio-rs/tokio)
- [NuShell](https://github.com/nushell/nushell)
- [Fig Completion Specs](https://github.com/withfig/autocomplete)
- [Warp Server Framework](https://github.com/seanmonstar/warp)
- [Alacritty](https://github.com/alacritty/alacritty)
- [Hyper HTTP library](https://github.com/hyperium/hyper)
- [FontKit](https://github.com/servo/font-kit)
- [Core-foundation](https://github.com/servo/core-foundation-rs)
- [Smol](https://github.com/smol-rs/smol)
