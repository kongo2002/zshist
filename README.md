# zshist

A shell history daemon-free replacement for zsh's built-in history. Stores
every command in a JSONL file with directory, exit code, timestamp, and
duration, and gives you fzf-powered fuzzy search plus prefix-based
up-arrow-style search.

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

## Data location

History is stored at `$HOME/.local/share/zshist/history.jsonl`. Each line is
a JSON object: timestamp, directory, exit code, command text, and duration
in milliseconds.

## Excluding commands

Commands matching `$HIST_EXCLUDE` (an array, same convention as
`HISTORY_IGNORE`-style exclude lists) are not recorded. Set it before
`zshist init` runs, e.g.:

```sh
HIST_EXCLUDE=(ls cd exit)
```
