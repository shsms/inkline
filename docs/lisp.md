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
  from 0 to 16, for bash code and inside the script of any command that
  uses a [command mode](#command-modes), whether or not its mode server
  can indent; `0` turns off both indenting new lines and moving closing
  words back out.
- `inkline-suggestion-lines` (default `5`): the most lines of a multi-line
  suggestion to show, an integer of at least 1. It applies only while no
  menu or message shows under the line; with one, a multi-line suggestion
  takes one row.
- `inkline-show-menu` (default `t`): whether the completion menu shows. A line
  brought back from history (with `C-p`, `<up>`, a search and the like) shows
  no menu and no grey text until you change it or press Tab. With no menu,
  `C-n` and `C-p` move between lines and search history by substring, as
  `<down>` and `<up>` do.
- `inkline-show-suggestion` (default `t`): whether the grey suggestion text
  shows.
- `inkline-menu-lines` (default `8`): the most rows the completion menu
  takes, an integer of at least 1.
- `inkline-completion-style` (default `orderless`): how the text you typed
  matches a completion item, the symbol `orderless` (the item holds each word
  you typed, split at blanks, anywhere and in any order), `substring` (the
  item holds what you typed anywhere, as one piece), `prefix` (the item starts
  with what you typed) or `fuzzy` (the letters you typed appear in the item in
  order, with gaps allowed). The items that start with what you typed come
  first, whatever their source. Then come, under `orderless`, the items that
  hold your words in the order you typed them, each after the one before, and
  then those that hold them in another order; under `substring`, the items
  that only hold it somewhere else; and under `fuzzy` the items that match
  with gaps, those whose letters lie closest together first. Items that match
  equally well go by source (see
  [`inkline-completion-functions`](#inkline-completion-functions)), and then
  in the source's own order (history newest first). A mode server's item that
  starts with a quote mark is also matched against the text after that mark
  when the mark goes on the line as it is: in an argument with an expansion
  (such as `$x`), and inside quotes of the other kind (`` ` `` or `"` inside
  single quotes, `'` inside double quotes). So inside single quotes `fi` finds
  `` `first name` ``.
- `inkline-completion-ignore-case` (default `nil`): when non-nil, the text you
  typed matches items whatever their case, in every style, so `am` also finds
  `Amount`. Items that start with the typed text in the same case come first,
  then those that start with it in another case, then the others. The grey
  text shows only when the top item starts with the typed text in the same
  case; taking an item from the menu puts the item's own case in the line.
- `inkline-menu-sources` (default `(history lisp mode bash)`): the sources the
  menu lists, a list of the symbols `history`, `lisp`, `mode` and `bash`. It
  limits only the menu: the grey text still comes from the top item of all the
  sources.
- `inkline-menu-min-chars` (default `0`): the menu lists an item only once you
  have typed at least this many characters of it (from where the item starts to
  the cursor: the whole line for a history item), an integer of at least 0. Like
  `inkline-menu-sources`, it limits only the menu.
- `inkline-menu-on-move` (default `nil`): when `nil`, a key that moves the
  cursor without changing the line's text hides the menu until the text
  changes or you press Tab, so `C-n`, `C-p` and the arrows then move between
  the lines of a command; when non-nil, the menu shows again wherever the
  cursor stops.
- `inkline-bash-completion` (default `t`): whether the menu gets items from
  bash's own completion, marked `c`: what bash's own Tab would offer for the
  word at the cursor, found in a copy of the shell. With `nil` no copy is
  started.
- `inkline-command-min-chars` (default `1`): how many characters of a command
  name you type before bash's items for it are listed, an integer of at least
  0. Other words get bash's items at once.
- `inkline-bash-completion-timeout` (default `2000`): how many milliseconds a
  copy of the shell may take to answer, an integer of at least 1; after that
  it is stopped and the word gets no `c` items.
- `inkline-history-cursor` (default `start`): the symbol `start` or `end`,
  where `previous-line-or-history`, `previous-line-or-substring-search` and
  `previous-line-or-search` leave the cursor in a multi-line entry they bring
  back, also when a menu key with no menu runs one of them. With `end`, an
  entry brought back by walking history keeps the cursor where readline puts
  it, and an entry the substring search finds has it at its end. An entry
  the prefix search finds has it after the prefix either way.
- `inkline-colors` (default `nil`): the colours inkline draws with, as an
  alist of `(NAME . "VALUE")` pairs, or a string in `LS_COLORS`'s format. A
  value is SGR codes or colour words, such as `"bold magenta"` or
  `"on grey4"`. The names are `command`, `unknown`, `keyword`, `option`,
  `string`, `variable`, `operator`, `comment`, `suggestion` and `error`;
  `separator`, for bash's `|` and `|&` in a pipeline and a `;` that ends a
  command and for a mode server's separators, drawn with the `operator`
  colour when it is not set; and three that only
  [command modes](#command-modes) use: `number` (default `36`), `function`
  (default `32`) and `script` (default `2`), the style added on top of
  every colour inside an argument a mode server coloured; and four for the
  completion menu: `menu`, for its rows (no colour by default),
  `menu-selected` (default `7`), for the highlighted row, `menu-source`
  (default `2`), for the source letter and the `… N more` row, and
  `menu-note` (default `2`), for an item's note; and `search-match`
  (default `7`), drawn on top of the colours over the text `C-r` or `C-s`
  matched in the command it found, in place of readline's
  `active-region-start-color` (the match of `M-p` and `M-n` keeps
  readline's colour); and `region` (default `7`), the active region
  (see the README's "What it does"; empty draws nothing, and the region
  still works). See the README's "Colours" section for the colour words.
- `inkline-command-mode-alist` (default `nil`): which commands use which
  [command mode](#command-modes).

## Keys

- `(keymap-global-set KEY COMMAND)` — binds `KEY`, in the emacs keymap, to
  `COMMAND`: a symbol naming a readline or inkline command (such as
  `'accept-line` or `'insert-pair`) or a Lisp function, or a lambda (see
  [Commands](#commands)). After `enable -d inkline`, and after an internal
  error turned inkline off until `inkline on`, a key bound to a Lisp command
  runs what it had before inkline first bound it (a readline command or macro
  text), or rings the bell when it had nothing. A key bound to `menu-next` or
  `menu-previous` runs, while no menu shows, what it had before inkline first
  bound it; readline's `next-history`, `previous-history` and substring
  searches, and nothing, become `next-line-or-substring-search` and
  `previous-line-or-substring-search`, and `history-search-forward` and
  `history-search-backward` become `next-line-or-search` and
  `previous-line-or-search` (with the `multi-line` group unbound, the key's
  own command runs as it is). When the key runs another of readline's history
  searches this way, the line that search finds shows no menu until you change
  it. `KEY` is an Emacs key description: keys separated by spaces. Each key is
  a character or one of `RET`, `TAB`, `DEL`, `SPC`, `ESC`, with an optional
  `M-` prefix (`M-RET`); a letter, `SPC`, or one of `@ [ \ ] ^ _ ?` may also
  take a `C-` prefix (`C-x`, `C-M-a`). A key may also be one of `<up>`,
  `<down>`, `<left>`, `<right>`, `<home>`, `<end>`, `<delete>`, `<backtab>`,
  which take no prefix. A named key stands for every sequence terminals send
  for it (four for `<home>` and `<end>`, two for each arrow), and a `KEY` may
  stand for at most 16 sequences. A `KEY` whose first keys run a command, such
  as `C-a C-b`, is refused. Binding `DEL`, `C-h`, `C-u`, `C-v` or `C-w` turns
  `bind-tty-special-chars` off. If `stty` gives the terminal other erase,
  kill, word-erase or literal-next characters, binding those keys does not;
  turn it off yourself with `inkline-set-readline-variable`.
- `(keymap-global-unset KEY)` — if `KEY` still runs what inkline bound it
  to, puts back what it ran before inkline took it over; a `bind` made
  since then is left alone. Binding `KEY` again while inkline still has it
  does not change what unset puts back.
- `(inkline-unbind-defaults &optional GROUPS)` — unsets the default layout's
  groups named in `GROUPS` (a symbol, or a list of symbols: `suggestions`,
  `multi-line`, `pairing`, `menu`, `region`); with no argument, unsets the
  whole layout.
  Unsetting `menu` while `multi-line` stays makes `C-n`, `C-p`, `<down>` and
  `<up>`, where they still run `menu-next` and `menu-previous`, run the
  command they run with no menu. Once inkline binds none of `DEL`, `C-h`,
  `C-u`, `C-v` and `C-w`, it turns `bind-tty-special-chars` back on, even
  when `inputrc` turned it off; call
  `(inkline-set-readline-variable "bind-tty-special-chars" nil)` after it to
  keep it off.
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
  `C-u` alone gives 4.
- `this-command` — the running command's own name; `nil` while a lambda
  runs.
- `last-command` — the name of the command that last ran from a key before
  this one (a readline or inkline command counts too, under the name
  inkline knows it by); `nil` if that one was a lambda, or this is the
  first command of the shell. A readline command that `call-interactively`
  ran on the current command's behalf does not become `last-command` on its
  own — only the Lisp command that called it does. After a count typed with
  `C-u` (`numeric-argument`), it names the command the count ran.
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
- Errors: an uncaught error in a command is shown under the line as `inkline:
  NAME: TEXT` (`lambda` in place of `NAME` for a lambda), and the line, point
  and mark go back to what they were. A bad setting value the command left is
  reported in the same message, after the error, with `; ` between them. `quit`
  shows nothing at all. `quit` happens when `C-c` arrives while the command runs
  — while it computes, as in an endless loop, or reads a key in `y-or-n-p` or
  inside a readline command run with `call-interactively` — or when a readline
  command run with `call-interactively` jumps back to readline's own top level
  on its own (such as `C-g`, or yanking with an empty kill ring), or runs shell
  code that fails in a way that makes bash drop the line (such as
  `shell-expand-line` on `${x:?}`, or `C-c` at a `read -e` in that shell code);
  once the command has stopped, bash then gives a new prompt as it would have
  without Lisp. No `condition-case` or `catch` catches a `quit`; see
  [Differences from Emacs](#differences-from-emacs).
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
  raises `quit` instead of running. `menu-next` and `menu-previous` run this
  way move once when the menu shows for the line and cursor as they are; the
  row stays when the command returns, and the moving ends there. Run from a
  command whose key comes right after a move, they end that moving and act
  as with no menu, as for a key that had nothing (see `keymap-global-set`).
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
in the list, in order, at certain times. The five hooks below start as `nil`.

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
`call-interactively` works in every hook except `inkline-suggestion-functions`
and `inkline-completion-functions`, but `call-interactively` of `undo`,
`revert-line` or `vi-undo` fails with an
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
  way, and shows nothing. `C-c` while a function runs, also at a `y-or-n-p`
  question, is a `quit`; bash then throws the line away and gives a new prompt,
  as `C-c` does.
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
  while a function runs, also in a readline command it runs with
  `call-interactively` (while that reads a key or runs shell code), is a `quit`,
  and bash then gives a new prompt, where the functions run again.
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
  other, as in Emacs. `this-command` is the name of the command the key ran
  (after a `C-u` count, the command the count ran), and `last-command` that of
  the key before (`nil` for a lambda, and at the first key of a line), so the
  functions can tell these apart. In the default
  layout the arrows run the menu commands, so Up that brings back a history
  entry gives `this-command` `menu-previous`, and Down `menu-next`. A key
  that ends an incremental search (`C-r`) runs its own command
  with `this-command` still naming the search, such as
  `reverse-search-history`.
- Their changes are one undo step of their own, after the key's: the first `C-_`
  takes back what they did and keeps what you typed.
- It runs after each move through the menu too, with `this-command` the move's
  command; a function that changes the line then ends the moving.
- Only called in plain editing: not while searching, reading a count (`M-3`,
  `C-u 3`) or a quoted key (`C-v`), and not when the key ran the line.
- A function that fails, also with `user-error`, has its own changes undone and
  is removed from the hook. `inkline: NAME: TEXT (removed from
  inkline-after-change-functions)` shows under the line. A `quit` undoes every
  change of this run, and no later function runs.

### `inkline-suggestion-functions`

Called with the line as a string whenever inkline gathers the completion
menu's items: the line is not empty, the cursor is at its end, and inkline
draws the line, in plain editing. Items are not gathered for a line brought
back from history until you change it or press Tab, nor while
`inkline-show-menu` and `inkline-show-suggestion` are both `nil`. No items are
gathered while moving through the menu.

- The first function that returns a string that starts with the line and is
  longer wins. Any other value is no answer, and the next function is asked.
- The winning answer becomes an item of the completion menu, marked `l`. A
  history entry that starts with the line in the same case still comes before
  it; one that starts with it only in another case, or only holds it, comes
  after it (see
  [`inkline-completion-functions`](#inkline-completion-functions) for the
  menu's order). An answer with a control character other than a newline or
  a tab is left out of the menu. The grey text after the cursor is the rest
  of the top item; where that is this answer, it shows as a history
  suggestion would, a newline giving a multi-line suggestion (see
  `inkline-suggestion-lines`). The same keys take it.
- Each answer, `nil` included, is kept for that exact line text until the next
  line starts. While what you type still matches the start of the suggestion, it
  stays, and the functions are not asked again.
- The functions may only read the line. Changing it, or moving point or the
  mark, raises "the line cannot be changed here". Calling one of these raises an
  error that names it, such as "message is not allowed in
  inkline-suggestion-functions": `call-interactively`, `y-or-n-p`, `message`,
  `print`, `princ`, `prin1`, `ding`, `kill-region`, `copy-region-as-kill`,
  `delete-char` with `KILLFLAG`, `keymap-global-set`, `keymap-global-unset`,
  `inkline-unbind-defaults`, `inkline-set-readline-variable` and
  `inkline-define-mode`.
- A function that fails, also with `user-error`, is removed from the hook.
  `inkline: NAME: TEXT (removed from inkline-suggestion-functions)` shows under
  the line. After a `quit`, no later function is asked, and that text of the
  line gets no suggestion.

### `inkline-completion-functions`

Called with no arguments whenever inkline gathers the completion menu's
items: at bash's main prompt while it reads a command, in plain editing, on
a line that is not empty and not brought back from history unchanged, and
not while `inkline-show-menu` and `inkline-show-suggestion` are both `nil`.
A function reads the line with `buffer-string`, `point` and the other
reading functions, and returns `nil` or a list `(START END ITEMS)`:

- `START` and `END` are positions around the cursor (`START <= (point) <=
  END`); an item taken from `ITEMS` replaces the line's text from `START` to
  `END`.
- `ITEMS` is a list of strings: every completion the function knows for that
  part of the line. inkline matches and orders them itself, against the
  text from `START` to the cursor, under `inkline-completion-style` and
  `inkline-completion-ignore-case`.
- Any other answer — `START` or `END` out of range or in the wrong order, or
  `ITEMS` not a list of strings — is an error, as for a function that fails.

Unlike Emacs's `completion-at-point-functions`, every function in the hook is
asked, and all their items go into the menu, marked `l`, in the order of the
hook and of each function's own list (`inkline-menu-sources` and
`inkline-menu-min-chars` can leave some of them out of the menu; an item left
out can still give the grey text). Items go by how well they match the typed
text (see `inkline-completion-style`), and those that match equally well by
source: history items first, then the item from
`inkline-suggestion-functions`, then a command's mode server's items, marked
`m` (see [command modes](#command-modes) and
[`docs/mode-protocol.md`](mode-protocol.md#completing-a-word-complete)), then
bash's own completion, marked `c` (see `inkline-bash-completion`), then these.

- The functions may only read the line, under the same rules as
  `inkline-suggestion-functions`, with the same list of functions they may
  not call; calling one of these raises an error naming
  `inkline-completion-functions` instead, such as "message is not allowed in
  inkline-completion-functions".
- A function that fails, also with `user-error`, or that gives a bad answer,
  is removed from the hook. `inkline: NAME: TEXT (removed from
  inkline-completion-functions)` shows under the line. After a `quit`, no
  later function is asked, and that line and cursor get no items from this
  hook.
- The functions are asked again only when the line's text or the cursor has
  changed since they were last asked; until then, the items they gave last
  time are used again.

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
- `(region-active-p)` — whether a region is active: true while inkline's
  region is active, or readline has an active mark of its own (an
  incremental search's match, pasted text).
- `(use-region-p)` — the same, but false for an empty region, as in Emacs.
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

## Command modes

A command mode gives the arguments of the commands that use it their own
colours, indentation and error checks, such as for the script in `csvm 'sort
id' data.csv`. Its mode server is the program that supplies them. See the
README's ["Command modes"](../README.md#command-modes), and
[`docs/mode-protocol.md`](mode-protocol.md) for writing a mode server.

- `(inkline-define-mode NAME PROGRAM &optional COLORS)` — defines the mode
  `NAME`, a symbol other than `nil` and `t`, with `PROGRAM` as its mode
  server. `PROGRAM` is a list of non-empty strings: the program, then its
  arguments, such as `'("csvm" "--inkline-mode")`. A program name without a
  `/` is looked up in bash's `PATH` when the server starts; `~` and
  variables in `PROGRAM` are not expanded. Returns `nil`. A `NAME` that is
  not such a symbol, or a `PROGRAM` that is neither such a list nor `nil`,
  is a `wrong-type-argument` error.
  - `COLORS` gives the mode colours of its own, in the forms
    `inkline-colors` takes: an alist of `(NAME . "VALUE")` pairs or a
    string in `LS_COLORS`'s format, such as `'((command . "bold magenta")
    (script . "on grey3"))`. The names are the ten kinds a mode server
    sends (`command`, `keyword`, `option`, `operator`, `string`, `number`,
    `variable`, `function`, `comment`, `separator`) and `script`. They are
    used only inside the arguments of the commands that use the mode, and
    a name left out uses `inkline-colors`. A `separator` set in neither
    takes the `operator` colour, found the same way. A value that cannot
    be read, or any other name, is an error (`inkline-define-mode: WHY`),
    in the string form too, and nothing changes. Left out or `nil`, the
    mode has no colours of its own.
  - Defining a mode again with the same `PROGRAM` keeps its running server
    and only changes its colours, from the next draw. A different `PROGRAM`
    replaces the server: the old process is stopped, and a new one starts
    the next time a line holds a command that uses the mode. Defining again,
    with any `PROGRAM`, turns back on a server that failed. To restart a
    running server, remove the mode, then define it again.
  - `(inkline-define-mode NAME nil)` removes the mode.
  - `inkline reload` forgets every mode and stops their servers; `init.el`
    then defines them again.
  - Not allowed in `inkline-suggestion-functions` (see
    [Hooks](#inkline-suggestion-functions)).
- `inkline-command-mode-alist` (default `nil`) — which commands use which
  mode: a list of `("COMMAND" . MODE)` pairs, `COMMAND` a string and `MODE`
  a symbol, such as `'(("csvm" . csvm-mode) ("c" . csvm-mode))`.
  - A command uses the first pair whose `COMMAND` is its name as typed,
    after quotes are removed, or the part of that name after its last `/`.
    So `"csvm"` covers `csvm`, `'csvm'`, `./target/debug/csvm` and
    `/usr/local/bin/csvm`, and a pair for `"./target/debug/csvm"` covers
    only that exact name. A name that bash will still change, such as
    `$prog`, uses no mode, and an empty `COMMAND` matches nothing. An alias
    is not expanded: `alias c=csvm` needs `("c" . csvm-mode)` too.
  - A pair whose `MODE` is not defined is skipped, so a later pair for the
    same command can match. So the variable may be set before the modes
    are defined.
  - `inkline status` lists each mode with the commands that use it, and
    each pair that never takes effect: one whose `MODE` is not defined
    (`command c: no mode named cvsm-mode`), one whose command an earlier
    pair already gives another mode (`command c: uses a-mode, not
    b-mode`), and one with an empty `COMMAND` (`command "": matches
    nothing`).
  - Several commands may use one mode: they share its one server and its
    colours.
  - It is read each time the line is drawn, like `inkline-colors`: a `setq`
    or `push` takes effect from the next draw. `add-to-list` skips a pair
    already in the list, since it compares with `member`, as in
    `(add-to-list 'inkline-command-mode-alist '("c" . csvm-mode))`; `push`
    works too, but adds its pair again each time it runs.
  - A value that is not a list of such pairs (a string `COMMAND` and a
    symbol `MODE` other than `nil` and `t`) is reported once
    (`inkline-command-mode-alist: expected a list of ("COMMAND" . MODE)
    pairs`), and no command uses a mode until the value changes.

## Other functions

- `(getenv NAME)` — the value of bash's variable `NAME`, exported or not,
  or `nil`.
- `(error FORMAT &rest ARGS)` — formats `FORMAT` and `ARGS` as `format`
  does, and raises a Lisp error with that text.
- `(user-error FORMAT &rest ARGS)` — like `error`, but shown as just its text,
  with no file, line or form. `condition-case` catches it as `user-error` or as
  `error`.

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
- `(fboundp SYMBOL)` — whether `SYMBOL` names a function, a macro or a special
  form.
- `(symbol-name SYMBOL)` — `SYMBOL`'s name, as a string.
- `(ignore &rest ARGS)` — does nothing; returns `nil`.
- `(identity X)` — returns `X`.
- `(zerop N)` — whether `N` is 0.
- `(defalias SYMBOL DEFINITION)` — makes `SYMBOL` name the function
  `DEFINITION`, such as a `lambda`, also for code that already calls it. A
  `DEFINITION` that names a function gives `SYMBOL` that function as it is now:
  `(defalias 'kar 'car)` works, but unlike in Emacs, `kar` does not follow a
  later change to `car`, and `car` must have a function already. A `SYMBOL` that
  `make-symbol` made takes only a function, not a macro.
- `(defconst SYMBOL VALUE)` — defines `SYMBOL` as a variable and sets it to
  `VALUE`.

### Control

- `(ignore-errors &rest BODY)` — a macro: runs `BODY`; if it signals an
  error, returns `nil` instead.

## Differences from Emacs

- No `condition-case` catches `quit`, whatever condition it names (`error`,
  `quit`, `t`, or anything else), and no `catch` does either. Only
  `unwind-protect` runs during it, and its cleanups get one second to finish.
  After that, a `C-c` stops them, and so does shell code that makes bash drop
  the line. This is true even for the `C-c` or shell code that made the `quit`.
  A jump back to readline's own top level, such as `C-g`, does not stop them.
- `split-string` splits on plain text, not a regular expression; with no
  `SEPARATORS`, it splits on runs of whitespace, as Emacs does.
- A name's global value and its function are one, unlike in Emacs: after
  `(defvar n 3)`, `(defun n () 'f)` makes the value of `n` that function.
- `push`'s `PLACE` must be a plain variable, not a general place such as a slot
  of a structure.
- `add-hook` does not know Emacs's hook depths: any non-`nil` `AT-END`,
  even a negative number, adds the function at the end.
- `string-to-number` with a `BASE` other than 10 reads no sign:
  `(string-to-number "-ff" 16)` is 0.
- `(interactive)` does nothing, `print` writes plain text, and there is no
  narrowing, so `point-min` is always 1; see [Commands](#commands) and
  [The line](#the-line).
