# zshist

[![zshist](https://github.com/kongo2002/zshist/actions/workflows/ci.yml/badge.svg)][actions]

A shell history daemon-free replacement for zsh's built-in history. Stores
every command in a JSONL file with directory, exit code, timestamp, and
duration, and gives you fzf-powered fuzzy search plus prefix-based
up-arrow-style search.

## Motivation

I am a big fan of a large shell history. However, having a large history file
significantly slows down the ZSH startup time. Therefore I always had to find a
"sweet spot" between having a large history and an acceptable startup time.

I wrote this tool, inspired by [zhist](https://github.com/overflowy/zhist),
because it is still very simple and does exactly what I need without a big
setup, for instance a daemon based approach would require.

## Features

- Per-directory and global history (`ctrl-g` toggles between them)
- fzf widget on `ctrl-r` with live preview of the full command
- Prefix search on `ctrl-p` / `ctrl-n` (like `up`/`down` history search, but
  cycling through matches instead of raw history order)
- Records exit code, working directory, and execution time per command
- Import existing zsh `EXTENDED_HISTORY` files
- Plain JSONL storage on disk, one entry per line

## Installation

Build the binary and put it on your `PATH`:

```sh
cargo install --path .
```

## Setup

Add this to your `.zshrc`:

```sh
eval "$(zshist init)"
```

This installs the `preexec`/`precmd` hooks that record commands, and binds
`ctrl-r`, `ctrl-p`, and `ctrl-n`. Requires [fzf](https://github.com/junegunn/fzf)
on `PATH` for the `ctrl-r` widget.

### Importing existing history

To migrate your current zsh history (must be in `EXTENDED_HISTORY` format,
i.e. `setopt EXTENDED_HISTORY`):

```sh
zshist import ~/.zsh_history
```

This only works against an empty zshist history — it refuses to import into
a history that already has entries.

### History settings

The following settings (or similar) are recommended for usage:

```sh
unset HISTFILE
HISTSIZE=100000
SAVEHIST=0

setopt append_history
setopt hist_allow_clobber
setopt hist_ignore_dups
setopt hist_expire_dups_first
setopt hist_ignore_space
setopt hist_verify
setopt no_extended_history
setopt no_inc_append_history
```

### Excluding commands

Commands matching `$HIST_EXCLUDE` (an array, same convention as
`HISTORY_IGNORE`-style exclude lists) are not recorded. Set it before
`zshist init` runs, e.g.:

```sh
HIST_EXCLUDE=(ls cd exit)
```

### Auto suggestions

In case you use the
[zsh-autosuggestions](https://github.com/zsh-users/zsh-autosuggestions) you want
to direct its strategy to `zshist` too:

```sh
_zsh_autosuggest_strategy_zshist() {
    suggestion=$(zshist search --limit 1 -- "$1")
}
ZSH_AUTOSUGGEST_STRATEGY=(zshist)
```

## Data location

History is stored at `$HOME/.local/share/zshist/history.jsonl`. Each line is
a JSON object: timestamp, directory, exit code, command text, and duration
in milliseconds.


[actions]: https://github.com/kongo2002/zshist/actions/
