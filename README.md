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
if enable -f ~/.local/lib/libinkline.so inkline; then
    # Suggestions: C-f / Right take a character, M-f a word, C-e / End all of
    # it. Without a suggestion these keys do what they always do.
    bind '"\C-f": accept-suggestion-char'
    bind '"\e[C": accept-suggestion-char'     # Right arrow
    bind '"\ef": accept-suggestion-word'
    bind '"\C-e": accept-suggestion'
    bind '"\e[F": accept-suggestion'          # End; some terminals send \eOF or \e[4~

    # Multi-line commands.
    bind '"\C-m": accept-or-newline'          # Enter
    bind '"\e\C-m": insert-newline'           # Alt+Enter
    bind '"\e[A": previous-line-or-history'   # Up
    bind '"\eOA": previous-line-or-history'
    bind '"\C-p": previous-line-or-history'
    bind '"\e[B": next-line-or-history'       # Down
    bind '"\eOB": next-line-or-history'
    bind '"\C-n": next-line-or-history'
    bind '"\C-a": line-start'
    bind '"\e[H": line-start'                 # Home; some terminals send \eOH or \e[1~
    bind '"\C-k": kill-to-line-end'
    bind '"\C-u": kill-to-line-start'
    bind '"\e#": comment-lines'
    bind 'set bind-tty-special-chars off'     # see below

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
    bind '"\C-?": delete-pair'                # Backspace

    bind 'set enable-bracketed-paste on'      # needed on bash 5.0 only
fi

# Keep multi-line commands whole in history (see "History" below).
shopt -s lithist
HISTTIMEFORMAT=
```

Put the bindings in `.bashrc`, inside the `if`, not in `~/.inputrc`. When
readline reads a binding to a command it does not know, it binds the key to
nothing: if inkline did not load, Enter would stop working.

If Up in your `inputrc` searches history (`history-search-backward`), bind Up
and Down to `previous-line-or-search` and `next-line-or-search` instead: they
move between lines the same way, and search past the first or last line.

`C-u` and Backspace need `bind-tty-special-chars off`: otherwise readline binds
the terminal's kill and erase characters back to `unix-line-discard` and
`backward-delete-char` every time it sets up the terminal. With it off,
readline also stops binding the terminal's kill, word-erase and literal-next
characters for you. Those default to `C-u`, `C-w` and `C-v`, which bash's
emacs bindings already cover, so this only matters if you changed them with
`stty`.

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
- **Multi-line commands.** Enter on an unfinished command (an open quote, a
  `for` without `done`, a trailing `|` or `\`) adds a line to it, indented to
  match; a closing word such as `done` moves back out when you press Enter.
  Enter on a finished command runs it, even one with a syntax error, so bash
  prints its usual message. Alt+Enter always adds a line; where Alt+Enter does
  not reach the shell (Windows Terminal uses it for fullscreen, macOS
  terminals need Option set as Meta), press Esc then Enter, or `C-v C-j`.
  `C-j` sends a command to bash as it is. Up and Down move between the lines
  and into history from the first and last line; a multi-line entry recalled
  with `previous-line-or-history` opens on its first line, so the next Up goes
  on through history. `C-a`, `C-e`, `C-k` and `C-u` act on the current line,
  and `C-k` and `C-u` join lines at its edges. `M-#` comments out every line.
  Pasted text keeps its own spacing.
- **Syntax errors.** A command bash would reject is underlined in wavy red
  when you pause typing, on the word bash would complain about. The word you
  are typing is never underlined.

While the terminal's echo is off, as in `read -e -s`, inkline leaves the line to
readline: no colours, no suggestion and no pairing.

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

`error` sets the syntax-error underline. By default it is a plain underline
(`4`) followed by a wavy red one (`4:3`, then `58:5:1`), so terminals that do
not know the wavy form still underline. `error=4` gives a plain underline, and
an empty `error=` turns it off. Sub-parameters with `:` are allowed in any
value.

## Settings

Plain shell variables, read each time they are used:

- `INKLINE_INDENT`: spaces per indentation step, 0 to 16, 4 by default and
  for any other value; `0` turns off both indenting new lines and moving
  closing words back out, and a closer put on its own line keeps its line's
  indentation.
- `INKLINE_HISTORY_CURSOR`: where `previous-line-or-history` leaves the cursor
  in a multi-line entry it recalls: `start` (the default) or `end`, where
  readline puts it.
- `INKLINE_SUGGESTION_LINES`: the most lines of a multi-line suggestion to
  show, 5 by default.

## History

By default bash joins the lines of a multi-line command with `;` when it saves
it, so it comes back on one line. With `shopt -s lithist` it keeps the
newlines. They survive into the next session only if bash writes timestamps
to the history file, which it does whenever `HISTTIMEFORMAT` is set, even to
an empty value. Every shell that writes the same file needs the setting.

Bash reads timestamps only if the file's first line is one, so convert an
existing history file once, with a timestamp before each old entry:

```bash
cp ~/.bash_history ~/.bash_history.bak
awk '!/^#[0-9]+$/ && prev !~ /^#[0-9]+$/ { print "#0" } { print; prev = $0 }' \
    ~/.bash_history.bak > ~/.bash_history
```

A block holding several commands (`echo one`, newline, `echo two`) is saved as
one entry per command. Bash also saves each line that `M-#` comments out as an
entry of its own, so a commented block comes back one line at a time.

## Commands

- `inkline on` / `inkline off`: switch highlighting, suggestions, the syntax
  underline and the multi-line commands; while off, the multi-line keys do
  what readline's own commands do. Pairing is only controlled by its
  bindings.
- `inkline status`: show whether inkline is on.
- `enable -d inkline`: remove the builtin. Bound keys keep working and behave
  like readline's own commands; `enable -f` loads inkline again.

## Limitations

- Where inkline cannot tell exactly where readline put each character, it leaves
  the line uncoloured and without suggestions, drawn by readline as usual: lines
  with control characters other than a newline or a tab (typed with `C-v`),
  lines taller than the terminal, `horizontal-scroll-mode`,
  `show-mode-in-prompt`, `mark-modified-lines`, a `PS1` with escape sequences
  outside `\[ \]`, a terminal readline has no cursor-up capability for (such
  as an unknown `TERM` over ssh), a locale that is not UTF-8, and, on bash
  5.1+, while readline highlights a search match or pasted text.
- A completion listing triggered in the same burst of typed-ahead keys as the
  text before it can leave grey suggestion text above the list.
- Bash can print while you type: a job notice when a background job ends under
  `set -b`, or a trap on a signal such as `WINCH`. When such a signal arrives
  while you type, inkline leaves the rest of that line to readline, which draws
  it without colours or a suggestion, even if nothing was printed. A job notice
  is written over the suggestion, and the end of a suggestion longer than the
  notice stays on screen. inkline does not notice a job that ends, or a trapped
  signal that arrives, while readline handles a key, and can then draw the rest
  of that line in the wrong place.
- A `bind -x` key pressed while a suggestion shows runs its command inside the
  synchronized update, so a terminal that supports it holds the command's output
  back until the command ends or the terminal's time limit for an update passes.
- A command taller than the terminal is drawn by readline without colours,
  and readline itself cannot draw it properly; Enter stops adding lines before
  a command gets that tall.
- Indentation counts from the left edge of the terminal, not from the end of
  the prompt.
- The syntax check cannot know aliases, and knows `extglob` only as it is set
  now: an alias or `shopt` set earlier in the same block is not seen. An error
  inside `$(…)` is underlined as bash 5.2 reports it, also on bash 5.0, which
  checks it only when it runs the command. A few rare forms get a wrong
  underline or keep Enter adding lines; `C-j` always sends the command to bash
  as it is.
- vi mode is not covered: the bindings above go into the emacs keymap.

## Testing

```sh
cargo test
scripts/build-bash.sh 5.0      # needs libncurses-dev / ncurses-devel
INKLINE_TEST_BASH=target/bash-5.0/bin/bash cargo test
scripts/build-bash.sh 5.3
INKLINE_TEST_BASH=target/bash-5.3/bin/bash cargo test
```

## License

GPL-3.0-only. See `LICENSE`.
