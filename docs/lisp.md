# inkline's Lisp reference

Everything this version of inkline adds to tulisp, its embedded Lisp
interpreter: the settings `init.el` can set, the functions it can call, and
where inkline's Lisp differs from Emacs's. See the [README](../README.md)
for how `init.el` is found and read, and for the default key layout.

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
  the readline or inkline command named by the symbol `COMMAND` (such as
  `'accept-line` or `'insert-pair`). `KEY` is an Emacs key description: keys
  separated by spaces. Each key is a character or one of `RET`, `TAB`,
  `DEL`, `SPC`, `ESC`, with an optional `M-` prefix (`M-RET`); a letter,
  `SPC`, or one of `@ [ \ ] ^ _ ?` may also take a `C-` prefix (`C-x`,
  `C-M-a`). A key may also be one of `<up>`, `<down>`, `<left>`, `<right>`,
  `<home>`, `<end>`, `<delete>`, `<backtab>`, which take no prefix. A named
  key stands for every sequence terminals send for it (four for `<home>` and
  `<end>`, two for each arrow), and a `KEY` may stand for at most 16
  sequences. A `KEY` whose first keys run a command, such as `C-a C-b`, is
  refused. Binding `DEL`, `C-h`, `C-u`, `C-v` or `C-w` turns
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
- `string-to-number` with a `BASE` other than 10 reads no sign:
  `(string-to-number "-ff" 16)` is 0.
- `?\s` reads as `?s` in tulisp — the character `s`, the number 115 — not
  the space character; write `? ` or `32` for a space instead.
