# inkline's Lisp reference

Everything this version of inkline adds to tulisp, its embedded Lisp
interpreter: the settings `init.el` can set, the functions it can call, the
hooks it can add functions to, and where inkline's Lisp differs from Emacs's.
See the [README](../README.md) for how `init.el` is found and read, and for the
default key layout.

## Settings

Set with `setq` in `init.el`; read again each time they are used, so a
change takes effect on the next key.

- `inkline-indent` (default `4`): spaces per indentation step, an integer
  from 0 to 16; `0` turns off both indenting new lines and moving closing
  words back out.
- `inkline-suggestion-lines` (default `5`): the most lines of a multi-line
  suggestion to show, an integer of at least 1.
- `inkline-history-cursor` (default `start`): the symbol `start` or `end`,
  where `previous-line-or-history` leaves the cursor in a multi-line entry
  it recalls.
- `inkline-colors` (default `nil`): the colours inkline draws with, as an
  alist of `(NAME . "SGR")` pairs, or a string in `LS_COLORS`'s format. See
  the README's "Colours" section.

## Keys

- `(keymap-global-set KEY COMMAND)` — binds `KEY`, in the emacs keymap, to
  `COMMAND`: a symbol naming a readline or inkline command (such as
  `'accept-line` or `'insert-pair`) or a Lisp function, or a lambda (see
  [Commands](#commands)). After `enable -d inkline`, and after an internal
  error turned inkline off until `inkline on`, a key bound to a Lisp command
  runs what it had before inkline first bound it (a readline command or
  macro text), or rings the bell when it had nothing. `KEY` is an Emacs key
  description: keys separated by spaces. Each key is a character or one of
  `RET`, `TAB`, `DEL`, `SPC`, `ESC`, with an optional `M-` prefix (`M-RET`);
  a letter, `SPC`, or one of `@ [ \ ] ^ _ ?` may also take a `C-` prefix
  (`C-x`, `C-M-a`). A key may also be one of `<up>`, `<down>`, `<left>`,
  `<right>`, `<home>`, `<end>`, `<delete>`, `<backtab>`, which take no
  prefix. A named key stands for every sequence terminals send for it (four
  for `<home>` and `<end>`, two for each arrow), and a `KEY` may stand for
  at most 16 sequences. A `KEY` whose first keys run a command, such as
  `C-a C-b`, is refused. Binding `DEL`, `C-h`, `C-u`, `C-v` or `C-w` turns
  `bind-tty-special-chars` off. If `stty` gives the terminal other erase,
  kill, word-erase or literal-next characters, binding those keys does not;
  turn it off yourself with `inkline-set-readline-variable`.
- `(keymap-global-unset KEY)` — if `KEY` still runs what inkline bound it
  to, puts back what it ran before inkline took it over; a `bind` made
  since then is left alone. Binding `KEY` again while inkline still has it
  does not change what unset puts back.
- `(inkline-unbind-defaults &optional GROUPS)` — unsets the default layout's
  groups named in `GROUPS` (a symbol, or a list of symbols: `suggestions`,
  `multi-line`, `pairing`); with no argument, unsets the whole layout. Once
  inkline binds none of `DEL`, `C-h`, `C-u`, `C-v` and `C-w`, it turns
  `bind-tty-special-chars` back on, even when `inputrc` turned it off; call
  `(inkline-set-readline-variable "bind-tty-special-chars" nil)` after it
  to keep it off.
- `(inkline-set-readline-variable NAME VALUE)` — sets the readline variable
  `NAME`, as `bind 'set NAME VALUE'` would. `VALUE` is a string, `nil` (for
  `off`), `t` (for `on`), or anything else, converted to text.

`COMMAND`, in `keymap-global-set` and `call-interactively`, can be a symbol
naming a readline command, an inkline command, or a Lisp function, or it can
be a lambda. For a symbol that names both a readline (or inkline) command
and a Lisp function, the Lisp function wins — except for eight names inkline
also uses for its own line-editing functions (`kill-region`, `delete-char`,
`forward-char`, `backward-char`, `beginning-of-line`, `end-of-line`,
`copy-region-as-kill`, `set-mark`): while the symbol still holds inkline's
own function under one of these names, binding it, or calling it with
`call-interactively`, runs the readline command of the same name instead.

## Commands

A command is a Lisp function, or a lambda, bound to a key with
`keymap-global-set`, or run with `call-interactively`. `(interactive)` is
accepted, as in Emacs, but it does nothing: a spec string or forms given to
it are ignored, and any function can be bound to a key or
run with `call-interactively`, with or without it. A command is always
called with no arguments — even one with `&optional` or `&rest` parameters
gets none — so it reads what it needs from `current-prefix-arg`, the line,
point and mark, not from arguments.

A key bound to a lambda runs that lambda object. A key bound to a named
symbol looks the function up by name each time the key runs, so it keeps
working after the symbol's function changes. A named command, once bound
to a key, is also added to readline's own commands under its name (unless
readline already has a command of that name, in which case a notice says
so and `bind` finds readline's instead), so `bind -q NAME` or `bind -p`
can find it too.

- `current-prefix-arg` — the count the key was pressed with, as an integer,
  for the length of the running command; `nil` when no count was given.
- `this-command` — the running command's own name; `nil` while a lambda
  runs.
- `last-command` — the name of the command that last ran from a key before
  this one (a readline or inkline command counts too, under the name
  inkline knows it by); `nil` if that one was a lambda, or this is the
  first command of the shell. A readline command that `call-interactively`
  ran on the current command's behalf does not become `last-command` on its
  own — only the Lisp command that called it does.
- Undo: everything a command changes about the line, however many editing
  functions it calls, undoes as one step. An error partway through, or
  `quit` (see below), puts the line, point and mark back to what they were
  before the command ran — unless the command moved to a different history
  entry, which is kept. `undo`, `vi-undo` and `revert-line`, run with
  `call-interactively`, first close the step built so far, so the
  command's own changes up to then count as one step, like each earlier
  command's: `undo` takes that step back first, and `revert-line` takes
  back every step. A later change in the same command starts a new step.
  An undo stays done even when the command then fails, since readline
  cannot redo. With a count of 0 or less, `undo` and `vi-undo` undo
  nothing and the step stays open. A command that moves to another
  history entry with `call-interactively` leaves an extra undo step on the
  line it left, and one that moves away, comes back and then fails keeps
  the change it made before it moved away; see Limitations in the
  [README](../README.md#limitations).
- Errors: an uncaught error in a command is shown under the line as
  `inkline: NAME: TEXT` (`lambda` in place of `NAME` for a lambda), and the
  line, point and mark go back to what they were. A bad setting value the
  command left is reported in the same message, after the error, with `; `
  between them. `quit` shows nothing at all. `quit` happens when `C-c`
  arrives while the command is reading a key — in `y-or-n-p`, or inside a
  readline command run with `call-interactively` — or when a readline
  command run with `call-interactively` jumps back to readline's own top
  level on its own (such as `C-g`, or yanking with an empty kill ring), or
  runs shell code that fails in a way that makes bash drop the line (such as
  `shell-expand-line` on `${x:?}`, or `C-c` at a `read -e` in that shell
  code); once the command has stopped, bash then gives a new prompt as it
  would have without Lisp. `quit` is a Lisp `throw`, and no `condition-case`
  catches a `throw`, whatever condition it names (`error`, `t`, or anything
  else) — only `unwind-protect` runs during it.
- Up to 256 Lisp commands — named functions and lambdas together — get a
  readline function of their own, so readline's `bind` can tell them
  apart. A key bound past that limit shares one function,
  `inkline-lisp-key`, which finds the right command by the key that was
  pressed; the key still runs the right command, but readline's own `bind`
  shows every such key as running `inkline-lisp-key`, not the command's own
  name (`inkline keys` still names each one correctly). Binding a named
  command past the limit prints a notice, unless another key already
  shares that same command through `inkline-lisp-key`; a lambda past the
  limit is shared with no notice. The same lambda object bound to more than
  one key uses one readline function. Once no key is bound to a lambda any
  more, its readline function is free for another command; a named command
  keeps its function for the life of the process, since readline keeps its
  name registered.
- `(call-interactively COMMAND)` — runs `COMMAND` (see above) the way pressing a
  key bound to it would, with `current-prefix-arg` as its count. Only works
  while a command or a hook function (see [Hooks](#hooks)) is already running. A
  Lisp command or lambda runs directly, with the same `current-prefix-arg`,
  `this-command` and `last-command` already in effect — it does not get its own.
  A readline or inkline command runs with the key the calling Lisp command or
  hook was run for; after a `C-c` came (while reading a key, while Lisp
  computed, or while a readline command ran), or shell code jumped as above, it
  raises `quit` instead of running.
- `(message FORMAT &rest ARGS)` — formats `FORMAT` and `ARGS` as `format`
  does and shows the text under the line until the next key, replacing any
  message already showing; `(message nil)` clears it. Only the text up to
  its first line break or other control character shows (a tab is kept).
  Under the line, it is cut to fit the screen's width; where it cannot go
  under the line (such as while inkline is off), it is printed in full
  above the prompt, and a long text takes more rows. Outside a command or a
  hook function, it goes to stderr instead, and `(message nil)` does
  nothing.
- `(print VALUE)`, `(princ VALUE)`, `(prin1 VALUE)` — show `VALUE` under the
  line the same way: `print` and `princ` write `VALUE` as plain text, as
  tulisp's `print` and `princ` do (`print` adds a newline, which does not
  show under the line; Emacs's `print` writes `VALUE` the way `prin1` does),
  and `prin1` writes `VALUE` as Lisp would read it back. Outside a command
  or a hook function, they write to stdout at once instead. All three
  return `VALUE`.
- `(y-or-n-p PROMPT)` — shows `PROMPT(y or n) ` under the line and reads one
  key: `y` or `Y` returns `t`, `n` or `N` returns `nil`, any other key rings
  the bell and asks again. `C-g`, `C-c`, or a key typed before it asks,
  raises `quit` instead of answering; keys that come from a readline macro
  (text bound to a key with `bind`) or a keyboard macro do answer it. Only
  works in a command or in `inkline-accept-functions`; elsewhere it is an
  error.
- `(ding)` — rings the bell.

## Hooks

A hook is a variable that holds a list of functions. inkline calls the functions
in the list, in order, at certain times. The four hooks below start as `nil`.

- `(add-hook HOOK FUNCTION &optional AT-END LOCAL)` — adds `FUNCTION` to the
  list in the variable `HOOK`: at the front, or at the end when `AT-END` is
  non-`nil`. A function already in the list (compared with `equal`) is not added
  again and stays where it is. `HOOK` does not need to exist first. `LOCAL` is
  ignored: inkline has no buffer-local variables. Returns the new list.
- `(remove-hook HOOK FUNCTION &optional LOCAL)` — removes every copy of
  `FUNCTION` (compared with `equal`) from `HOOK`. `LOCAL` is ignored. Returns
  `nil`.

As in Emacs, a hook may hold a single function instead of a list; `add-hook`
then turns it into a list. A `t` in the list is skipped.

Hooks run only at bash's main prompt while it reads a command, and only while
inkline is on. They do not run in `read -e`, at bash's `>` prompt (where it asks
for the rest of an unfinished command), while inkline is off, or while Lisp is
already running. Changes that hook functions make never run
`inkline-after-change-functions`.

In every hook, `current-prefix-arg` is `nil`. So are `this-command` and
`last-command`, except in `inkline-after-change-functions`.
`call-interactively` works in every hook except `inkline-suggestion-functions`,
but `call-interactively` of `undo`, `revert-line` or `vi-undo` fails with an
error such as `undo cannot run in a hook`. `y-or-n-p` works only in
`inkline-accept-functions`; in the line-start and after-change hooks it fails
with `y-or-n-p works only in a command or in inkline-accept-functions`.

A readline command run with `call-interactively` gets the key that ran the hook,
so `(call-interactively 'self-insert)` inserts the key that runs the line in
`inkline-accept-functions` (a carriage return, for Enter), and nothing in
`inkline-line-start-functions`, which no key runs.

In the messages below, `NAME` is the function's name, or `lambda`; when more
than one function fails, the messages under the line are joined with `; `.

### `inkline-accept-functions`

Called with no arguments just before a line runs: when Enter
(`accept-or-newline`) or `M-RET` (`accept-as-is`) runs it, or `C-j`
(`insert-newline`) while a macro replays or where inkline adds no lines (with
`horizontal-scroll-mode` on, or on a terminal that cannot move the cursor up).
Also called for an empty line.

- The functions see the line as typed, before bash's history and alias
  expansion. The line runs as they leave it, even if it is now unfinished: bash
  then asks for the rest at its `>` prompt. A line they changed is drawn again
  before it runs, and history keeps the line as it ran.
- `(user-error …)` refuses the line: every change of this run is undone, its
  text shows under the line on its own (with no `inkline: NAME:`), and the line
  stays for editing. No later function runs. A `quit` refuses the line the same
  way, and shows nothing. `C-c` at a `y-or-n-p` question is a `quit`; bash then
  throws the line away and gives a new prompt, as `C-c` does.
- Any other error undoes that function's own changes, and prints `inkline: NAME:
  TEXT` on a row of its own above the command's output, so it stays in the
  scrollback. The next function runs, and the line still runs.
- A `message` does not stay on screen once the line runs.
- Not called while a `C-c` waits for bash, which throws that line away. Not
  called when `C-o` (`operate-and-get-next`), `M-#` (`comment-lines`) or a key
  bound with `bind` to readline's `accept-line` runs the line.

### `inkline-line-start-functions`

Called with no arguments when a new command line begins at the main prompt,
before you type. The line may already hold text: after `C-o`, the next history
line.

- What they change is one undo step. Point stays where they leave it, and the
  line is drawn again when they changed it. The suggestion and
  `inkline-after-change-functions` start from the line as they leave it.
- A function that fails, also with `user-error`, has its own changes undone but
  stays in the hook. `inkline: NAME: TEXT` shows under the line, and the next
  function runs.
- A `quit` undoes every change of this run, and no later function runs. `C-c`
  in a readline command a function runs with `call-interactively` (while it
  reads a key or runs shell code) is a `quit`, and bash then gives a new prompt,
  where the functions run again.
- If a function never ends (an endless loop) at the first line of a shell, the
  next shell skips `init.el`, as it does for an `init.el` that never finishes
  (see the README). A function at the first line that waits for a key (in a
  readline command run with `call-interactively`) for more than ten seconds
  counts as stuck too: a new shell started meanwhile skips `init.el`.

### `inkline-after-change-functions`

Called with `(BEG END OLD-LEN)`, as in Emacs, once after each key whose command
changed the line: after the command has returned, and before the line is drawn.

- `BEG` and `END` are the positions of the changed text in the line as it is
  now; `OLD-LEN` is how many characters that part had before. It is the smallest
  part that differs, and several changes in one key make one part. When the
  change could be in more than one place, as when a space is typed before
  another space, it is the place nearest the cursor: where it was before the key
  or after it, whichever is further left.
- The line is compared with the line after the last run of this hook, or of the
  line-start hook: no difference, no call. Keys that readline handles in one go,
  such as typed-ahead characters it inserts together, or a macro, can be one
  change, as a paste is.
- History recall, a search, undo and taking a suggestion are changes like any
  other, as in Emacs. `this-command` is the name of the command the key ran, and
  `last-command` that of the key before (`nil` for a lambda, and at the first
  key of a line), so the functions can tell these apart. On bash 5.3, a key
  that ends an incremental search (`C-r`) runs its own command with
  `this-command` still naming the search, such as `reverse-search-history`.
- Their changes are one undo step of their own, after the key's: the first `C-_`
  takes back what they did and keeps what you typed.
- Only called in plain editing: not while searching, reading a count (`M-3`) or
  a quoted key (`C-v`), and not when the key ran the line.
- A function that fails, also with `user-error`, has its own changes undone and
  is removed from the hook. `inkline: NAME: TEXT (removed from
  inkline-after-change-functions)` shows under the line. A `quit` undoes every
  change of this run, and no later function runs.

### `inkline-suggestion-functions`

Called with the line as a string when inkline would show a history suggestion
but history has none: the line is not empty, the cursor is at its end, and
inkline draws the line, in plain editing. History always wins over these
functions.

- The first function that returns a string that starts with the line and is
  longer wins. Any other value is no answer, and the next function is asked.
- The rest of that string shows after the cursor as a history suggestion would:
  cut at the first control character other than a newline or a tab, with a
  newline giving a multi-line suggestion of at most `inkline-suggestion-lines`
  lines. The same keys take it.
- Each answer, `nil` included, is kept for that exact line text until the next
  line starts. While what you type still matches the start of the suggestion, it
  stays, and the functions are not asked again.
- The functions may only read the line. Changing it, or moving point or the
  mark, raises "the line cannot be changed here". Calling one of these raises an
  error that names it, such as "message is not allowed in
  inkline-suggestion-functions": `call-interactively`, `y-or-n-p`, `message`,
  `print`, `princ`, `prin1`, `ding`, `kill-region`, `copy-region-as-kill`,
  `delete-char` with `KILLFLAG`, `keymap-global-set`, `keymap-global-unset`,
  `inkline-unbind-defaults` and `inkline-set-readline-variable`.
- A function that fails, also with `user-error`, is removed from the hook.
  `inkline: NAME: TEXT (removed from inkline-suggestion-functions)` shows under
  the line. After a `quit`, no later function is asked, and that text of the
  line gets no suggestion.

## The line

These functions read and change the command line being edited — the line
readline shows and the user is typing, which may hold more than one
terminal row. Positions are 1-based character counts, the way Emacs counts
them (inkline itself works in bytes internally); point 1 is before the
first character, and `(point-max)` is one past the last one.

- `(point)` — the position of point.
- `(point-min)` — always 1: there is no narrowing.
- `(point-max)` — one past the line's last character.
- `(goto-char POS)` — moves point to `POS`, clamped to the line; returns
  `POS` as given, even when it had to clamp.
- `(forward-char &optional N)`, `(backward-char &optional N)` — moves point
  by `N` characters (1 by default), backward for the second; an error, with
  point left where it was, if that would move past either end.
- `(beginning-of-line)`, `(end-of-line)` — moves point to the start or end
  of its line (a multi-line command has more than one).
- `(forward-line &optional N)` — moves point to the start of the line `N`
  lines on (back, for a negative `N`; the current line's start, for 0).
  Never an error, even past either end of the text — it returns how many
  lines short of `N` the move fell (0 for a complete move).
- `(bolp)`, `(eolp)` — whether point is at the start or end of its line.
- `(bobp)`, `(eobp)` — whether point is at the very start or end of the
  whole line being edited.
- `(line-beginning-position)`, `(line-end-position)` — the position of the
  start or end of point's line.
- `(current-column)` — point's screen column, counted from its line's
  start; a tab goes to the next multiple of 8, other characters count
  their display width.
- `(char-after &optional POS)`, `(char-before &optional POS)` — the
  character after or before `POS` (point, by default), or `nil` at the end
  or start of the line, and when `POS` is outside the line.
- `(mark)` — the position of the mark.
- `(set-mark POS)` — moves the mark to `POS`, clamped to the line.
- `(region-beginning)`, `(region-end)` — the smaller or larger of point and
  the mark.
- `(use-region-p)`, `(region-active-p)` — the same thing: whether readline
  currently has an active mark. Needs readline 8.1 or later (bash 5.1 and
  later); on bash 5.0 they are always `nil`.
- `(skip-chars-forward SPEC &optional LIM)`, `(skip-chars-backward SPEC
  &optional LIM)` — moves point over the characters matching `SPEC`,
  stopping at `LIM` (the line's other end, by default; clamped to the line
  and to point, not an error). Returns how many characters it moved over
  (negative from `skip-chars-backward`). `SPEC` is plain characters and
  `a-z`-style ranges, with a leading `^` to negate the set; there is no
  backslash escaping or named character class.
- `(buffer-string)` — the whole line, as one string.
- `(buffer-substring START END)`, `(buffer-substring-no-properties START
  END)` — the same: the text between `START` and `END`, in either order; an
  error if either falls outside the line.
- `(insert &rest ARGS)` — inserts each of `ARGS` at point (a string as
  itself, an integer as one character) and leaves point after what it
  inserted; an error if the result would contain a NUL character.
- `(delete-region START END)` — deletes the text between `START` and `END`,
  in either order; an error if either falls outside the line.
- `(delete-char N &optional KILLFLAG)` — deletes `N` characters after point
  (before point, for a negative `N`); keeps the deleted text for a later
  yank when `KILLFLAG` is non-`nil`, joined to the previous kill as
  `kill-region` does, in front of it for a negative `N`. An error,
  deleting nothing, if that would run past either end.
- `(erase-buffer)` — deletes the whole line.
- `(kill-region START END)` — deletes the text between `START` and `END`
  and keeps it for a later yank. As readline's own kill commands do, it
  joins the text to the previous kill when the command run before it, or
  an earlier kill in the same command, also killed: after the previous
  kill, or in front of it when `END` is before `START`. An error if either
  position falls outside the line.
- `(copy-region-as-kill START END)` — keeps the text between `START` and
  `END` for a later yank, without deleting it, joined to the previous kill
  as `kill-region` does; an error if either position falls outside the
  line.
- `(save-excursion &rest BODY)` — a macro: runs `BODY`, then puts point
  back where it was, even if `BODY` signals an error or `quit`. It saves
  and restores only point, not the mark.

Out of range: `forward-char`, `backward-char`, `delete-char`,
`buffer-substring`, `buffer-substring-no-properties`, `delete-region`,
`kill-region` and `copy-region-as-kill` raise an `args-out-of-range` error
and change nothing when a position falls outside the line. `goto-char`,
`set-mark`, `skip-chars-forward` and `skip-chars-backward` clamp an
out-of-range position to the line instead of raising an error, and
`char-after` and `char-before` give `nil` for it. A position that is not a
number is always a `wrong-type-argument` error, from any of these functions.

Inserting text moves the mark: one after the insertion point moves along
with the inserted text; one at or before it stays. Deleting text moves
point and the mark back: one at or after the deleted range moves back by
its length; one inside the range moves to the start of the range; one
before the range stays. `save-excursion`'s saved point follows the same
rules, so it still lands in the right place after an insert or delete
inside its body. A readline command run with `call-interactively` inside
the body does not move the saved point; if that command made the line
shorter, point comes back at most at the line's end, and at the start of
the character the saved point now falls inside.

Outside a line being edited (from `init.el`, `inkline eval`, or `inkline
load`), the reading functions above see an empty line, with point, mark and
`point-max` all 1. Moving point or the mark does nothing and is not an
error. As on an empty line, `forward-char`, `backward-char` and
`delete-char` with a count other than 0 raise `args-out-of-range`, and so do
`buffer-substring`, `buffer-substring-no-properties`, `delete-region`,
`kill-region` and `copy-region-as-kill` with a position other than 1. Every
function that changes the line's text, and `copy-region-as-kill`, raises "no
line is being edited" instead. While `inkline-suggestion-functions` runs,
the line is read-only: see [Hooks](#hooks). When the line being edited is
not valid UTF-8 (text in another encoding was typed or pasted), every
function above raises "the line is not UTF-8" and changes nothing.

## Other functions

- `(getenv NAME)` — the value of bash's variable `NAME`, exported or not,
  or `nil`.
- `(error FORMAT &rest ARGS)` — formats `FORMAT` and `ARGS` as `format`
  does, and raises a Lisp error with that text.
- `(user-error FORMAT &rest ARGS)` — like `error`, but shown as just its
  text, with no file, line or form. `condition-case` catches it only as
  `error`, not as `user-error`, and the message text a handler sees starts
  with `user-error` between two NUL characters.

## Emacs functions inkline adds

tulisp lacks these; inkline defines them so they behave as Emacs's do.

### Strings

- `(substring STRING &optional FROM TO)` — the characters of `STRING` from
  `FROM` (default 0) to `TO` (default the end); a negative count is from
  the end.
- `(string-prefix-p PREFIX STRING &optional IGNORE-CASE)` — whether
  `STRING` starts with `PREFIX`.
- `(string-suffix-p SUFFIX STRING &optional IGNORE-CASE)` — whether
  `STRING` ends with `SUFFIX`.
- `(string-search NEEDLE STRING &optional START)` — the character position
  of the first `NEEDLE` in `STRING` at or after `START`, or `nil`.
- `(split-string STRING &optional SEPARATORS OMIT-NULLS)` — `STRING` cut at
  each run of whitespace, or at each occurrence of the plain text
  `SEPARATORS`; `OMIT-NULLS` drops the empty pieces.
- `(string-trim STRING)` — `STRING` with leading and trailing spaces, tabs
  and newlines removed.
- `(string-replace FROM TO STRING)` — `STRING` with every plain-text `FROM`
  replaced by `TO`.
- `(string-empty-p STRING)` — whether `STRING` has no characters.
- `(string-to-number STRING &optional BASE)` — the number at the start of
  `STRING`, after leading blanks, or 0 if there is none. `BASE` is from 2 to
  16; a `BASE` other than 10 (the default) reads whole numbers only, not
  fractions or exponents.
- `(number-to-string NUMBER)` — `NUMBER` written as a string.
- `(upcase X)`, `(downcase X)` — `X`, a string or a character, in upper
  case (`upcase`) or lower case (`downcase`).

### Characters

- `(char-to-string CHAR)` — `CHAR` as a one-character string.
- `(string &rest CHARS)` — `CHARS` joined into a string.
- `(string-to-char STRING)` — the first character of `STRING`, or 0 if it
  is empty.

### Lists and symbols

- `(push NEWELT PLACE)` — a macro: sets the variable `PLACE` to a new list
  with `NEWELT` added to the front. `PLACE` must be a plain variable, not a
  general place such as a slot of a structure.
- `(pop PLACE)` — a macro: removes and returns the first element of the
  list held in the variable `PLACE`.
- `(add-to-list LIST-VAR ELEMENT &optional APPEND)` — adds `ELEMENT` to the
  front of the list in `LIST-VAR` (the end, with `APPEND`) unless it is
  already a member; returns the list.
- `(assq KEY ALIST)` — the first pair of `ALIST` whose `car` is `eq` to
  `KEY`, or `nil`.
- `(delete ELT SEQ)`, `(remove ELT SEQ)` — `SEQ` without the elements
  `equal` to `ELT`.
- `(delq ELT LIST)` — `LIST` without the elements `eq` to `ELT`.
- `(mapc FUNCTION SEQUENCE)` — calls `FUNCTION` on each element of
  `SEQUENCE`, for effect; returns `SEQUENCE`.
- `(nreverse SEQ)` — `SEQ` reversed.
- `(car-safe X)`, `(cdr-safe X)` — the `car`/`cdr` of `X`, or `nil` if `X`
  is not a cons.
- `(fboundp SYMBOL)` — whether `SYMBOL` names a function.
- `(symbol-name SYMBOL)` — `SYMBOL`'s name, as a string.
- `(ignore &rest ARGS)` — does nothing; returns `nil`.
- `(identity X)` — returns `X`.
- `(zerop N)` — whether `N` is 0.
- `(defalias SYMBOL DEFINITION)` — makes `SYMBOL` name the function
  `DEFINITION`. `DEFINITION` must be a function itself, such as a `lambda`;
  a symbol that names a function does not work: `(defalias 'kar 'car)`
  itself succeeds, but calling `kar` then fails with "function is void:
  car".
- `(defconst SYMBOL VALUE)` — defines `SYMBOL` as a variable and sets it to
  `VALUE`.

### Control

- `(ignore-errors &rest BODY)` — a macro: runs `BODY`; if it signals an
  error, returns `nil` instead.

## Differences from Emacs

- `split-string` splits on plain text, not a regular expression; with no
  `SEPARATORS`, it splits on runs of whitespace, as Emacs does.
- `format` — and so `error` and `user-error` — supports only `%s`, `%S`,
  `%d` and `%f`.
- A name is either a function or a variable, not both:
  `(let ((list 5)) (list 1))` fails, unlike in Emacs.
- `add-hook` does not know Emacs's hook depths: any non-`nil` `AT-END`,
  even a negative number, adds the function at the end.
- `string-to-number` with a `BASE` other than 10 reads no sign:
  `(string-to-number "-ff" 16)` is 0.
- `(interactive)` does nothing, `print` writes plain text, and there is no
  narrowing, so `point-min` is always 1; see [Commands](#commands) and
  [The line](#the-line).
- `?\s` reads as `?s` in tulisp — the character `s`, the number 115 — not
  the space character; write `? ` or `32` for a space instead.
