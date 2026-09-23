# inkline

Syntax highlighting, history suggestions and bracket/quote pairing for bash,
without replacing readline. Every key and `inputrc` setting keeps working
exactly as before: inkline only changes how the line is drawn and adds new
readline commands for you to bind.

It is a bash loadable builtin written in Rust, for bash 5.0 and later.

## Building

Needs a Rust toolchain and a C compiler (for tree-sitter).

```sh
cargo build --release
mkdir -p ~/.local/lib
cp target/release/libinkline.so ~/.local/lib/
```

## Setup

Add to `~/.bashrc`:

```bash
enable -f ~/.local/lib/libinkline.so inkline

# Suggestions: C-f / Right take a character, M-f a word, C-e / End all of
# it. Without a suggestion these keys do what they always do.
bind '"\C-f": accept-suggestion-char'
bind '"\e[C": accept-suggestion-char'     # Right arrow
bind '"\ef": accept-suggestion-word'
bind '"\C-e": accept-suggestion'
bind '"\e[F": accept-suggestion'          # End; some terminals send \eOF or \e[4~

# Pairing (optional).
bind '"(": insert-pair'
bind '"[": insert-pair'
bind '"{": insert-pair'
bind '"\"": insert-pair'
bind "\"'\": insert-pair"
bind '"`": insert-pair'
bind '")": insert-close'
bind '"]": insert-close'
bind '"}": insert-close'
bind 'set bind-tty-special-chars off'     # see below
bind '"\C-?": delete-pair'                # Backspace

bind 'set enable-bracketed-paste on'      # needed on bash 5.0 only
```

Put the bindings in `.bashrc`, after `enable -f`, not in `~/.inputrc`.  When
readline reads an `inputrc` line naming a command it does not know, it binds the
key to nothing, so a bash without inkline (or one that read `inputrc` before
loading inkline) would lose those keys entirely.

Backspace needs `bind-tty-special-chars off`: otherwise readline binds the
terminal's erase character back to `backward-delete-char` every time it sets up
the terminal, replacing `delete-pair`. With it off, readline also stops binding
the terminal's kill, word-erase and literal-next characters for you. Those
default to `C-u`, `C-w` and `C-v`, which bash's emacs bindings already cover, so
this only matters if you changed them with `stty`.

## What it does

- **Highlighting.** Commands, keywords, options, strings, variables, operators
  and comments are coloured as you type. A command that is not a keyword, alias,
  function, builtin or program on `PATH` is red.
- **Suggestions.** When the cursor is at the end of the line, the newest history
  entry starting with what you typed is shown in grey after the cursor. Accepted
  text can be undone with `C-_`.
- **Pairing.** `(`, `[`, `{` and quotes insert their closing character; typing
  the closer moves over it; Backspace between an empty pair deletes both. It
  stays out of the way after letters and digits (`don't`), after a backslash,
  inside comments and strings, and with a count prefix. Pasted text is never
  paired.

Each keystroke's output is sent as one synchronized update (DEC private mode
2026), so terminals that support it, such as alacritty, kitty, foot and WezTerm,
show a single frame per key. Terminals without it ignore the markers, and there
the grey text and new characters can flicker briefly as readline and inkline
draw in turn.

## Colours

Set `INKLINE_COLORS` in the same format as `LS_COLORS`. Any SGR codes work,
including 256-colour and truecolor. List only what you want to change; changes
apply immediately.

```bash
INKLINE_COLORS='command=32:unknown=31:keyword=35:option=36:string=33:variable=34:operator=1:comment=2:suggestion=90'
```

## Commands

- `inkline on` / `inkline off`: switch highlighting and suggestions.  Pairing is
  only controlled by its bindings.
- `inkline status`: show whether inkline is on.
- `enable -d inkline`: remove the builtin. Bound keys keep working and behave
  like readline's own commands; `enable -f` loads inkline again.

## Limitations

- Each line is highlighted on its own; continuation lines (after `PS2`) do not
  know about the lines before them.
- Where inkline cannot tell exactly where readline put each character, it leaves
  the line uncoloured and without suggestions, drawn by readline as usual: lines
  with control characters (typed with `C-v`), lines taller than the terminal,
  `horizontal-scroll-mode`, `show-mode-in-prompt`, `mark-modified-lines`, a
  `PS1` with escape sequences outside `\[ \]`, a terminal readline has no
  cursor-up capability for (such as an unknown `TERM` over ssh), a locale that
  is not UTF-8, and, on bash 5.1+, while readline highlights a search match or
  pasted text.
- A completion listing triggered in the same burst of typed-ahead keys as the
  text before it can leave grey suggestion text above the list.
- A `bind -x` key pressed while a suggestion shows runs its command inside the
  synchronized update, so a terminal that supports it holds the command's output
  back until the command ends or the terminal's time limit for an update passes.

## Testing

```sh
cargo test
scripts/build-bash-5.0.sh      # needs libncurses-dev / ncurses-devel
INKLINE_TEST_BASH=target/bash-5.0/bin/bash cargo test
```

## License

GPL-3.0-only. See `LICENSE`.
