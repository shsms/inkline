# inkline

Syntax highlighting, history suggestions and bracket/quote pairing for bash,
without replacing readline. inkline binds its own layout of keys when it
loads, but only on a key that still has readline's default binding for it —
`inputrc`, and your own `bind` lines after `enable -f`, win.
`(inkline-unbind-defaults)` in `init.el` gives every key in the layout back
to readline.

It is a bash loadable builtin written in Rust, for bash 5.0 and later,
configured with `init.el`, a small Emacs Lisp file (see "Configuration"
below).

## Building

Needs a Rust toolchain and a C compiler (for tree-sitter). Until tulisp has its
own release, cargo fetches it from its git repository, at a fixed revision.

```sh
make install    # builds and copies libinkline.so to ~/.local/lib
```

## Setup

Add to `~/.bashrc`:

```bash
enable -f ~/.local/lib/libinkline.so inkline

# Keep multi-line commands whole in history (see "History" below).
shopt -s lithist
HISTTIMEFORMAT=
```

That is all `.bashrc` needs. On load, inkline binds its own layout of keys —
the table below — but only on a key that still has readline's default
binding for it, so it never undoes a change you already made. Put any `bind`
lines of your own after `enable -f`; they run after inkline's and win.

| Group | Key | Command | Default it needs |
|---|---|---|---|
| suggestions | `C-f`, `<right>` | `accept-suggestion-char` | `forward-char` |
| suggestions | `M-f` | `accept-suggestion-word` | `forward-word` |
| suggestions | `C-e`, `<end>` | `accept-suggestion` | `end-of-line` |
| multi-line | `RET` | `accept-or-newline` | `accept-line` |
| multi-line | `C-j` | `insert-newline` | `accept-line` |
| multi-line | `M-RET` | `accept-as-is` | unbound or `vi-editing-mode` |
| multi-line | `<up>`, `C-p` | `previous-line-or-history` | `previous-history` |
| multi-line | `<down>`, `C-n` | `next-line-or-history` | `next-history` |
| multi-line | `C-a`, `<home>` | `line-start` | `beginning-of-line` |
| multi-line | `C-k` | `kill-to-line-end` | `kill-line` |
| multi-line | `C-u` | `kill-to-line-start` | `unix-line-discard` |
| multi-line | `M-#` | `comment-lines` | `insert-comment` |
| pairing | `(` `[` `{` `"` `'` `` ` `` | `insert-pair` | `self-insert` |
| pairing | `)` `]` `}` | `insert-close` | `self-insert` |
| pairing | `DEL` | `delete-pair` | `backward-delete-char` |

`<right>`, `<end>`, `<home>`, `<up>` and `<down>` are also taken when they are
unbound (many terminals send sequences for them that plain bash never binds
on its own); `M-RET` is taken when it is unbound or bound to
`vi-editing-mode`. If `inputrc` binds `<up>`/`<down>` to
`history-search-backward`/`history-search-forward`, inkline binds
`previous-line-or-search`/`next-line-or-search` there instead: they move
between lines the same way, and search past the first or last line.

Turn part of the layout off in `init.el`:

```elisp
;; ~/.config/inkline/init.el
(inkline-unbind-defaults '(pairing))   ; no bracket pairing
```

`(inkline-unbind-defaults)` with no argument gives back the whole layout; a
single group name also works, as `(inkline-unbind-defaults 'pairing)`.

`C-u` and Backspace need `bind-tty-special-chars off`: otherwise readline binds
the terminal's kill and erase characters back to `unix-line-discard` and
`backward-delete-char` every time it sets up the terminal. inkline turns it off
the first time it loads in a shell and on `inkline reload`, along with
`enable-bracketed-paste on` on readline 8.0, and again whenever Lisp binds
`DEL`, `C-h`, `C-u`, `C-v` or `C-w`; `inkline-unbind-defaults` turns it back on
once inkline binds none of them, even when `inputrc` turned it off. `inputrc`
can still turn `bind-tty-special-chars` back on, but only when inkline is the
first thing to set readline up — that is, only when nothing runs `bind` before
`enable -f` in `.bashrc`. When it can, an `inputrc` that turns
`bind-tty-special-chars` back on makes readline rebind Backspace and `C-u` to
the terminal's erase and kill characters on every line, undoing `delete-pair`
and `kill-to-line-start`. With it off, readline also stops binding the
terminal's kill, word-erase and literal-next characters for you. Those default
to `C-u`, `C-w` and `C-v`, which bash's emacs bindings already cover, so this
only matters if you changed them with `stty`.

## Configuration

inkline reads `$XDG_CONFIG_HOME/inkline/init.el` if `XDG_CONFIG_HOME` is set
to an absolute path, else `~/.config/inkline/init.el`, once when it loads in
an interactive shell with line editing on (`bash --noediting -i` reads
nothing). It is read only when it is a regular file, it and its
directory belong to you (or to root), and neither is writable by group or
others; otherwise inkline prints one line saying why it was skipped (a
missing file prints nothing). When `init.el` is a link, as dotfile managers
make it, the file it points at, that file's directory and the directory
holding each link on the way from `init.el` to that file must pass these
checks too. Directories higher up, and links to directories, are not
checked.

tulisp reads the whole file, then compiles all of it, then runs it. A parse
error, such as a missing `)`, means nothing in the file runs or is defined. A
compile error, such as a known function called with the wrong number of
arguments, means nothing runs, but the functions defined above the mistake
exist. A runtime error stops the file at that form: the forms above it have
run, and every function the file defines exists, even ones below it. Either
way inkline prints one line; it does not undo what already ran.

Before reading `init.el`, inkline leaves a marker in `$XDG_STATE_HOME/inkline`
(or `~/.local/state/inkline`), and removes it as soon as `init.el` has been
read. If a shell's `init.el` never finishes — an endless loop, for example —
its marker stays. A later shell that finds it, once the stuck shell has ended
or the marker is more than 10 seconds old, skips `init.el` until the file
changes, and says so; fix the file, then run `inkline reload`. The same
happens when a function in `inkline-line-start-functions` never finishes at a
shell's first line (see [`docs/lisp.md`](docs/lisp.md#hooks)). The marker
directory gets the same owner and permission checks as `init.el`'s
directory. If it fails them, inkline prints one line saying so and reads
`init.el` without markers, so a looping `init.el` is not skipped.

Example `init.el`:

```elisp
;; ~/.config/inkline/init.el
(setq inkline-indent 2)
(setq inkline-colors '((command . "32") (string . "33")))
(keymap-global-set "C-x C-r" 'accept-as-is)
```

A command is a Lisp function bound to a key with `keymap-global-set`:

```elisp
;; ~/.config/inkline/init.el
(defun upcase-line ()
  (interactive)
  (let ((text (upcase (buffer-string))))
    (erase-buffer)
    (insert text)))
(keymap-global-set "C-x u" 'upcase-line)

(defun confirm-clear ()
  (interactive)
  (when (y-or-n-p "Clear the line? ")
    (erase-buffer)))
(keymap-global-set "C-x k" 'confirm-clear)
```

`C-x u` now upper-cases the whole line; `C-x k` asks under the line and
clears it on `y`.

A hook is a variable holding a list of functions that inkline calls at certain
times; `add-hook` adds one. An abbreviation can turn into its full text when you
type a space after it, with a command bound to `SPC`, and when the line runs,
with a function in `inkline-accept-functions`:

```elisp
;; ~/.config/inkline/init.el
(defvar my-abbrevs '(("gst" . "git status") ("ll" . "ls -l")))

;; Replaces the word before the cursor when it is an abbreviation.
(defun my-expand ()
  (let* ((end (point))
         (start (save-excursion (skip-chars-backward "^ \n") (point)))
         (full (cdr (assoc (buffer-substring start end) my-abbrevs))))
    (when full
      (delete-region start end)
      (insert full))))

(defun expand-then-space ()
  (interactive)
  (my-expand)
  (call-interactively 'self-insert))
(keymap-global-set "SPC" 'expand-then-space)

(add-hook 'inkline-accept-functions
          (lambda () (goto-char (point-max)) (my-expand)))
```

`gst` then Space gives `git status `; `ll` then Enter runs `ls -l`.

A function in `inkline-suggestion-functions` gets the line and returns a longer
line to suggest, or `nil`. inkline asks it only when history has no suggestion.
It may read the line, but not change it:

```elisp
;; ~/.config/inkline/init.el
(defvar my-commands '("make test" "git log --oneline"))

(add-hook 'inkline-suggestion-functions
          (lambda (line)
            (let ((found nil))
              (dolist (command my-commands)
                (when (and (not found) (string-prefix-p line command))
                  (setq found command)))
              found)))
```

Typing `make t` now shows `est` in grey after the cursor.

Every variable, function, command and hook inkline adds to Lisp is listed in
[`docs/lisp.md`](docs/lisp.md).

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
  match. In bash code, but not inside a string or a here-document, the spaces
  and tabs around the cursor go, so the new line starts with just its
  indentation. A closing word such as `done` moves back out when you press
  Enter.
  Enter on a finished command runs it, even one with a syntax error, so bash
  prints its usual message. `C-j` adds a line, even to a finished command; where
  inkline adds no lines, such as at bash's `> ` prompt, in `read -e` or while
  inkline is off, it sends the line, as in plain bash. A `C-j` that readline
  replays from a macro, such as the `\n` in `"\C-xr": "echo hi\n"` or one
  recorded with `C-x (`, also runs the command. `M-RET` (Alt+Enter) runs
  `accept-as-is`, which sends the command to bash as it is, finished or not,
  after the accept hook (`inkline-accept-functions`); where Alt+Enter does
  not reach the shell (Windows Terminal uses it for fullscreen, macOS
  terminals need Option set as Meta), press Esc then Enter. An Enter typed
  while a command is still running arrives as `C-j`, so it adds a line to the
  next command instead of running it, like pasted text. Up and Down move
  between the lines and into history from the first and last line; a
  multi-line entry recalled with `previous-line-or-history` opens on its
  first line, so the next Up goes on through history. `C-a`, `C-e`, `C-k`
  and `C-u` act on the current line, and `C-k` and `C-u` join lines at its
  edges. `M-#` comments out every line. Pasted text keeps its own spacing.
- **Syntax errors.** A command bash would reject is underlined in wavy red
  when you pause typing, on the word bash would complain about. The word you
  are typing is never underlined.
- **Program arguments.** A program can colour its own arguments, such as the
  script in `csvm 'sort id | head 5' data.csv`, and underline its own errors
  in them; see "Colouring a program's arguments" below.

While the terminal's echo is off, as in `read -e -s`, inkline leaves the line to
readline: no colours, no suggestion and no pairing.

Each keystroke's output is sent as one synchronized update (DEC private mode
2026), so terminals that support it, such as alacritty, kitty, foot and WezTerm,
show a single frame per key. Terminals without it ignore the markers, and there
the grey text and new characters can flicker briefly as readline and inkline
draw in turn.

## Colouring a program's arguments

Some programs take a small language as an argument, such as the script in
`csvm 'select amount > 1000 | sort id' data.csv`. inkline can ask the
program itself how to colour it. The program must come with a highlight
helper: a mode that answers inkline's questions, such as `csvm
--highlight`. Name the command and its helper in `init.el`:

```elisp
;; ~/.config/inkline/init.el
(inkline-highlight-arguments "csvm" '("csvm" "--highlight"))
```

Then, wherever a `csvm` command is on the line (in a pipeline, after `&&`,
inside `$( … )`, on any line of a multi-line command):

- The parts of the script get colours, as the program's own parser sees
  them: its commands, operators, numbers, column names and so on.
- The whole script is dim, so it stands apart from the rest of the command.
  The quote marks keep bash's string colour, so you can see where the script
  starts and ends. A variable such as `$min` inside a double-quoted script
  keeps bash's colour.
- An error the program finds, such as a mistyped command or an unknown
  column, is underlined like a syntax error when you pause typing, and
  `csvm: MESSAGE` shows under the line for as long as the underline does. An
  error with no place in the arguments shows only its message. The word you
  are typing is never underlined. There is no error in an argument that
  bash will still change, such as one with a `$x`, a `*` or a leading `~`,
  and no error when bash sees a syntax error on the line: then bash's error
  is underlined instead.

The `script` colour key sets the style added on top of every colour inside
such a script. It is dim (`2`) by default; a background colour is the other
look:

```elisp
;; ~/.config/inkline/init.el
(setq inkline-colors '((script . "on grey4")))   ; a dark grey background
```

`(script . "")` turns it off. The helper's colours use the usual keys, and
`number` and `function` (see "Colours" below).

A third argument to `inkline-highlight-arguments` gives the command colours
of its own, in the forms `inkline-colors` takes. They are used only inside
that command's arguments, for the nine kinds a helper sends (`command`,
`keyword`, `option`, `operator`, `string`, `number`, `variable`, `function`
and `comment`) and for `script`; a name they leave out uses
`inkline-colors`. Bash's own colours inside the script, such as the colour
of a `$min`, of the quote marks and of the error underline, stay as
`inkline-colors` sets them.

```elisp
;; ~/.config/inkline/init.el
(inkline-highlight-arguments "csvm" '("csvm" "--highlight")
  '((command . "bold magenta") (variable . "cyan") (number . "yellow")
    (script . "on grey3")))
```

A bad colour, or a name other than those ten, is an error, and nothing
changes. Registering the command again with the same program keeps its
helper running and only changes the colours, from the next draw; leaving the
third argument out drops them.

The command is found by its name as typed, after quotes are removed, so
`'csvm'` matches too. Its arguments are its words as bash will pass them to
the program: a redirection such as `2>err` and `VAR=x` words before the name
are left out, and words after a redirection still count. An alias is not
expanded, so it needs its own line: after `alias c=csvm`, add
`(inkline-highlight-arguments "c" '("csvm" "--highlight"))`.

inkline starts the helper the first time a line holds the command, and
finds its program through bash's `PATH`. The helper then runs in the
background until the shell exits. inkline waits at most 15 ms for its answer
each time it draws the line; an answer that comes later is drawn as soon as
it comes, or, while readline asks a question (such as whether to show every
completion), once you have answered. `inkline status` shows one line for
each helper:

```
highlight csvm: running
```

or `highlight csvm: not started`, or `highlight csvm: off (REASON)`. A
helper that fails is turned off, and `inkline: highlight csvm: off (REASON)`
shows under the line when you pause typing. The reason is `not found`,
`cannot run: …`, `not a highlight helper` (the program did not answer as a
helper), `bad reply: …` (it broke the protocol, or wrote too much),
`exited`, or `connection lost` (a command in the shell closed inkline's end
of the connection). It stays off until it is registered again or you run
`inkline reload`. `inkline reload` stops every helper; `init.el` registers
them again.

Helpers run only at bash's main prompt, including every line of a
multi-line command. They do not run at bash's own `>` prompt (`PS2`, where
bash asks for the rest of an unfinished command), in `read -e`, or while
inkline is off.

inkline talks to each running helper over a socket, and keeps its end on a
high file descriptor: the highest free one below 256, where bash keeps its
own. So each running helper takes one such descriptor, and the ones you
redirect, such as `exec 3>file` or `exec 10>file`, stay yours.

Two limits:

- A command that has a `case` or a heredoc (`<<`) inside a `$( … )`, `<( … )`
  or `>( … )` in its arguments, or that is itself inside backquotes (`` `…`
  ``), gets no colours from its helper: inkline cannot tell for sure where
  its words end, or what bash will make of them.
- A subshell that bash forks while a helper runs, such as `while :; do sleep
  100; done &`, keeps the helper's connection open. A helper stopped by
  `inkline reload` or by registering the command again with another program
  keeps running until that subshell ends.

To write a helper for your own program, see
[`docs/highlight-protocol.md`](docs/highlight-protocol.md).

## Colours

Set `inkline-colors` in `init.el`, either as an alist of `(NAME . "VALUE")`
pairs or as a string in `LS_COLORS`'s format. A value is SGR codes (any work,
including 256-colour and truecolor) or colour words (see below). List only
what you want to change; a change applies from the next key.

```elisp
(setq inkline-colors '((command . "32") (unknown . "31") (keyword . "35")
                        (option . "36") (string . "33") (variable . "34")
                        (operator . "1") (comment . "2") (suggestion . "90")
                        (number . "36") (function . "32") (script . "2")))
```

or, in the old string format:

```elisp
(setq inkline-colors "command=32:unknown=31:keyword=35:option=36:string=33:variable=34:operator=1:comment=2:suggestion=90:number=36:function=32:script=2")
```

In the alist form, a name can be a symbol or a string, and the first entry
for a name wins. `""` means no colour for that name, and turns the underline
off for `error`.

A value can be words instead of codes, such as `"bold magenta"`, `"on grey5"`
or `"underline bright-cyan on black"`. A value made only of digits, `;` and
`:` is read as SGR codes, so older settings keep working. The words are
separated by spaces, and upper or lower case does not matter:

- `bold`, `dim`, `italic`, `underline` and `reverse`;
- a colour for the text:
  - `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan` or
    `white`, or the same with `bright-` in front, such as `bright-red`;
  - `grey`, the same as `bright-black`;
  - `grey0` to `grey23`, the 24 greys of the 256-colour palette, from the
    darkest, almost black (`grey0`), to the lightest, almost white
    (`grey23`); `grey3` to `grey6` make a dark background;
  - `color0` to `color255` (or `colour0` to `colour255`), any colour of
    the 256-colour palette;
  - `#rrggbb`, any colour, on terminals that show 24-bit colour, such as
    `#ff8700`;
- `on` and one of those colours, for the background: `on grey3`,
  `on #3a3a3a`, `on bright-blue`.

`gray` works wherever `grey` does. In the alist form, a word inkline does not
know is reported, with the word, and one bad entry makes inkline use its
default colours for everything until it is fixed; the string form skips just
that entry.

```elisp
(setq inkline-colors '((command . "bold green") (comment . "dim italic")
                        (script . "on grey3")))
```

`number`, `function` and `script` are used only by highlight helpers (see
"Colouring a program's arguments"). `number` and `function` are colours a
helper can give parts of an argument. `script` is added on top of every
colour inside an argument a helper coloured: dim (`2`) by default, or a
background such as `on grey4`.

`error` sets the syntax-error underline. By default it is a plain underline
(`4`) followed by a wavy red one (`4:3`, then `58:5:1`), so terminals that do
not know the wavy form still underline. `error=4` (or `(error . "4")`) gives a
plain underline, and `error=` (or `(error . "")`) turns it off.
Sub-parameters with `:` are allowed in any value written as codes.

## Settings

Lisp variables, set in `init.el` with `setq` and read each time they are
used, so a change takes effect on the next key. A value of the wrong type or
out of range is reported once, and the default is used until the value
changes.

- `inkline-indent`: spaces per indentation step, an integer from 0 to 16, 4
  by default; `0` turns off both indenting new lines and moving closing
  words back out, and a closer put on its own line keeps its line's
  indentation.
- `inkline-history-cursor`: where `previous-line-or-history` leaves the
  cursor in a multi-line entry it recalls: the symbol `start` (the default)
  or `end`, where readline puts it.
- `inkline-suggestion-lines`: the most lines of a multi-line suggestion to
  show, an integer of at least 1, 5 by default.

See [`docs/lisp.md`](docs/lisp.md) for every setting, function and command
inkline adds to Lisp.

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
  bindings. Lisp commands still run while inkline is off; a message they
  show goes on a row of its own above the prompt.
- `inkline status`: two lines — whether inkline is on, then `init.el`'s
  status: loaded, not found, skipped (with why), failed (with the error), or
  not read (because the shell is not interactive with line editing on, or
  has no usable `HOME`); then one line for each highlight helper (see
  "Colouring a program's arguments").
- `inkline load FILE`: read and run a file of Lisp. An error is printed to
  stderr and the command exits 1.
- `inkline eval EXPR`: run one Lisp expression, given as a single argument;
  prints its value with `prin1` unless the value is `nil`. An error is
  printed to stderr and the command exits 1.
- `inkline reload`: start a fresh interpreter — unbind the keys inkline or
  `init.el` bound, stop every highlight helper, bind the default layout
  again (by the same rules as at load), and read `init.el` again. A `bind`
  you ran yourself is left alone.
  If `init.el` is skipped or has an error, the reason is printed and the
  command exits 1.
- `inkline keys`: one line for each key inkline has bound and what it runs,
  then one line for each layout key left alone because it no longer had
  readline's default, and what has it instead.
- `enable -d inkline`: remove the builtin. Keys bound to inkline's commands
  keep working and behave like readline's own commands; a key bound to a
  Lisp command runs what it had before inkline first bound it, or rings the
  bell when it had nothing. `enable -f` loads inkline again. A second
  `enable -f` in the same shell only switches inkline back on: it does not
  read `init.el` again and does not bind the default layout again; use
  `inkline reload` for that.

## Upgrading

If you set inkline up before this version: delete the old `bind` block from
`.bashrc`. inkline's layout now binds those keys itself, with the same
commands except `M-RET`: the block bound it to readline's `accept-line`, the
layout binds it to `accept-as-is`. The old `M-RET` line would skip the
accept hook (`inkline-accept-functions`), as readline's `accept-line` does.
The block runs after `init.el`, so it would also bind again any key that
`init.el` gives back or binds to something else. Move any
`INKLINE_COLORS`, `INKLINE_INDENT`, `INKLINE_HISTORY_CURSOR` or
`INKLINE_SUGGESTION_LINES` value into `init.el`, as `inkline-colors`,
`inkline-indent`, `inkline-history-cursor` and `inkline-suggestion-lines`.
Until you do, inkline still starts, but prints one line per variable still
set and does not read any of them.

The layout binds every group, pairing included, even if you never bound it
before. Enter now runs `accept-or-newline`, so Enter on an unfinished command
adds a line. Give back the groups you do not want in `init.el`, for example
`(inkline-unbind-defaults '(pairing))` or `(inkline-unbind-defaults
'(multi-line))`. `inkline status` now prints a second line, about
`init.el`; a script that checks its output should read only the first
line.

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
  underline or keep Enter adding lines; Alt+Enter always sends the command to
  bash as it is.
- vi mode is not covered: `keymap-global-set` and the default layout only
  bind in the emacs keymap.
- Readline commands that look at the command run before them — `yank-pop`,
  `yank-last-arg` pressed again, a history search that goes on from the last
  one — see a Lisp command as that command, not what it ran with
  `call-interactively`; inside a Lisp command, they see the key pressed
  before it. So `M-y` after a Lisp command that yanked rings the bell.
- `kill-region` adds its text to the previous kill, as readline's own kill
  commands do, when the key pressed before the Lisp command killed text, or
  the same command killed text before; Emacs does so only after a command
  that killed.
- After `enable -d inkline`, or after an internal error until `inkline on`,
  a key bound to a Lisp command runs what it had before inkline first bound
  it, or rings the bell when it had nothing.
- A Lisp command that moves to another history entry with
  `call-interactively`, such as `(call-interactively 'previous-history)`,
  leaves an extra undo step on the line it left. Text and history are
  kept, and a change the command made there stays. Back on that line, undo
  works as usual until it reaches the point where the command started:
  there one `C-_` only rings the bell, and the next goes on. `history`
  marks that entry with `*`, as it does an edited one. A command that
  moves away, comes back and then fails gets point and the mark put back,
  and keeps the change it made before it moved away. Moving through
  history with keys is not affected.
- An endless loop in Lisp freezes the shell: there is no way yet to stop it
  with `C-c`. Closing the terminal window ends the shell. The next shell
  skips a looping `init.el` and says so; `bash --norc` starts a shell that
  does not read `.bashrc`, and so does not load inkline, giving you a shell
  to fix `init.el` in.
- A tool that drives an interactive bash by sending the keys it wants typed,
  followed by `\n`, adds a line instead of running the command: the default
  layout binds Enter to `accept-or-newline`, and `\n` is `C-j`, which always
  adds a line. Send `\r` instead, start bash with `--norc`, or turn
  multi-line editing off with `(inkline-unbind-defaults '(multi-line))`.
- `inkline-after-change-functions` and `inkline-suggestion-functions` run
  on almost every key you type, so keep their functions small and fast: a
  slow one makes typing slow.
- These ways of running a line skip the accept hook
  (`inkline-accept-functions`): `C-o` (`operate-and-get-next`), `M-#`
  (`comment-lines`), and a key bound with `bind` to readline's `accept-line`.
- A key bound with `keymap-global-set` that starts a longer sequence, such as
  `C-x` or `ESC`, runs only after readline's `keyseq-timeout` has passed, or
  as soon as a key arrives that does not continue the sequence.
- Lisp can crash bash by recursing too deep: printing a list nested about
  15,000 levels deep, dropping one nested about 85,000 levels deep (with
  bash's usual 8 MB stack), or calling `equal` on two circular lists.

## Testing

```sh
make test       # with the system bash
make test-all   # also with bash 5.0 and 5.3, built into target/ first
make check      # fmt, clippy and the unit tests
```

Building the older bashes needs libncurses-dev / ncurses-devel. To test
with one bash, set `INKLINE_TEST_BASH`:
`INKLINE_TEST_BASH=target/bash-5.0/bin/bash cargo test`.

## License

GPL-3.0-only. See `LICENSE`.
