# Kuru Homebrew tap

Install the current verified native release:

```sh
brew install replygirl/kuru/kuru
kuru --provider demo
```

Update through Homebrew:

```sh
brew update
brew upgrade kuru
```

Supported targets are Apple Silicon macOS and GNU/Linux ARM64 or x86-64 with
glibc compatible with Ubuntu 24.04. Intel macOS is unsupported. The executable
includes the complete pinned Dolt engine and its licenses; it needs no separate
database installation or first-run engine download. Bash, Zsh, Fish and
PowerShell completions and the manual page are installed in Homebrew's standard
directories. Follow [Homebrew shell setup](https://docs.brew.sh/Shell-Completion)
to load the completions for your shell.

`Formula/kuru.rb` is generated and accepted in
[Kuru's source repository](https://github.com/replygirl/kuru). Its single manual
Release workflow verifies the immutable public release before updating this tap
with a dedicated repository-scoped GitHub App. The tap does not publish releases
or build another executable. Retry a failed tap update on the original release
run; identical formula content creates no new commit, and older or conflicting
versions are refused.

See [installation](https://github.com/replygirl/kuru/blob/main/docs/install.md)
and [release operations](https://github.com/replygirl/kuru/blob/main/docs/release.md).
