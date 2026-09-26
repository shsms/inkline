# The inkline mode protocol

A mode server is a program that supplies a command mode: it tells inkline
how to colour the arguments of the commands that use the mode, what errors
they hold, and how far in a new line of a script goes. It is usually the
command's own program, run in a special mode, such as `csvm --inkline-mode`:
it parses the arguments with the program's own parser, so the colours always
match the language. This page is for people who write one. See the README's
["Command modes"](../README.md#command-modes) for how a user sets one up, and
[`docs/lisp.md`](lisp.md#command-modes) for `inkline-define-mode` and
`inkline-command-mode-alist`.

This is version 1 of the protocol. All numbers are decimal ASCII. Lengths
and offsets count bytes, not characters: `é` is two bytes. Every line ends
with one newline byte (`\n`), never `\r\n`. Fields on a line are separated by
single spaces.

## How inkline runs a mode server

- inkline starts the mode server the first time a line holds a command that
  uses its mode, with the program and arguments given to
  `inkline-define-mode`. One server answers for every command that uses its
  mode. A program name without a `/` is looked up in bash's `PATH`. The
  server's environment is what bash exports at that moment, as for a
  program bash runs then.
- The server's stdin and stdout are one socket, connected to inkline. Its
  stderr is `/dev/null`, so anything it writes there is lost; to debug,
  write to a file of your own.
- The server is not bash's child: bash's `jobs` and `wait` never see it. It
  runs in a session of its own, so `C-c` at the prompt does not reach it.
- The server keeps running while the shell does, and answers one request
  after another. When bash exits, or the server is stopped (`inkline
  reload`, `enable -d inkline`, its mode defined again with another program
  or removed with `nil`, or the server turned off for one of the reasons
  below), inkline closes its end of the socket, and the server reads the
  end of its input. There is no signal: the end of input is the only sign
  to exit. A subshell that bash forked while the server ran, such as
  `while :; do sleep 100; done &`, holds a copy of inkline's end, so the
  server then sees the end of its input only once that subshell ends too.
- The server starts in `/`, so it never keeps a directory in use. Run the
  program's own parser on the text the program would get, and use the
  directory in each request (`:cwd`) to find files: that is the shell's
  directory at the time of the request.

## The first line

As soon as it starts, the server writes this line on its stdout:

```
inkline-mode 1
```

The line may name extra requests the server can answer, as words after the
`1`, each after a single space: `inkline-mode 1 indent`. inkline accepts
the words, and only ever sends a server the kinds of request it named.
Version 1 defines one extra request, `indent` (see "Indenting a new line"
below). A server that does not answer it sends the line with no words;
inkline ignores words it does not know. Any other first line turns the
server off (`not a mode server`).

## A request

inkline writes a request on the server's stdin:

```
:request ID
:cwd LEN
BYTES
:arg KIND LEN
BYTES
…one :arg for each argument…
:done
```

- `ID` counts up from 1, for each server process. Colour requests and
  indent requests share the count.
- `:cwd` gives the shell's current directory (bash's `PWD`); it is empty
  when `PWD` is unset.
- There is one `:arg` for each word of the command, in order. Argument 0 is
  the command's name as typed, after quotes are removed, such as `csvm`,
  `c` or `./target/debug/csvm`: several commands may use one mode, so a
  server should not count on one name.
- `BYTES` is exactly `LEN` bytes, then one newline, which is not part of it.
  The bytes may hold anything, newlines included, so read `LEN` bytes, not a
  line.
- `KIND` is `final` or `raw`:
  - `final`: the argument exactly as the program will get it, after bash
    removes the quotes and backslashes.
  - `raw`: bash will still change the argument before the program gets it,
    so `BYTES` is the text as typed, quote marks included. This is an
    argument that holds a `$` (a variable, `$( … )`, `$(( … ))`, `$'…'`,
    `$"…"`) or a backquote outside single quotes; or, outside all quotes, a
    glob (`*`, `?`, `[`, or an extglob such as `@(…)`), a brace expansion
    (`{a,b}`, `{1..5}`), a leading `~` (also after the `=` of a word such
    as `x=~/a`, or after a later `:` in such a word, as in `PATH=a:~/b`),
    or a process substitution (`<( … )`, `>( … )`).

Redirections, such as `>out` or `2>&1`, and `VAR=x` words before the command
name are not arguments. inkline sends a request only when it has none in
flight to that server, colour or indent, so a server never has more than
one request to answer at a time.

## A reply

The server answers on its stdout:

```
:span ARG START END KIND
:error ARG START END MESSAGE
:end ID
```

- Zero or more `:span` lines, in any order. `ARG` is the argument's index.
  `START` and `END` are byte offsets into that argument's `BYTES`, with
  `START < END <= LEN`. `KIND` is one of `command`, `keyword`, `option`,
  `operator`, `string`, `number`, `variable`, `function`, `comment` and
  `separator`; each is drawn with the `inkline-colors` key of the same
  name. `separator` is for a mark between parts, such as the `|` between
  a pipeline's stages; when the user has not set its colour, it is drawn
  with the `operator` colour.
- At most one `:error` line, with `START <= END` and `END <= LEN`. `MESSAGE`
  is the rest of the line after the space that follows `END`: one line of
  text. The space is there even when `MESSAGE` is empty (`:error 1 0 2 `);
  without it the line does not parse. An error that has no place in the
  arguments is written `:error - - - MESSAGE`.
- `:end ID` ends the reply, with the `ID` of the request it answers.

## An example

The user types, in `/home/me/sales`:

```bash
csvm 'sort id | head 5' data.csv
```

inkline sends this request, the first of several: it sends one each time
the arguments or the directory change, as the user types. Each line below
ends with one `\n` byte, and there are no other bytes; the whole request is
111 bytes:

```
:request 1
:cwd 14
/home/me/sales
:arg final 4
csvm
:arg final 16
sort id | head 5
:arg final 8
data.csv
:done
```

Argument 1 is `sort id | head 5`, without its quote marks: that is what csvm
gets. The server answers:

```
:span 1 0 4 command
:span 1 5 7 variable
:span 1 8 9 separator
:span 1 10 14 command
:span 1 15 16 number
:end 1
```

`:span 1 0 4 command` is bytes 0 to 4 of argument 1, `sort`. inkline maps
the offsets back to what was typed, so `sort` on the line is drawn in the
`command` colour, just after the quote mark.

Next the user mistypes the column as `idd`. The request is the same,
except for `:request 2` and the argument (now 17 bytes):

```
:arg final 17
sort idd | head 5
```

and the server answers with an error on bytes 5 to 8, `idd`:

```
:span 1 0 4 command
:span 1 5 8 variable
:span 1 9 10 separator
:span 1 11 15 command
:span 1 16 17 number
:error 1 5 8 unknown column 'idd'
:end 2
```

A `raw` argument is sent as typed. For `csvm "head $n" data.csv`, argument 1
is:

```
:arg raw 9
"head $n"
```

Its offsets count from the first `"`. inkline never paints the quote marks,
and `$n` keeps bash's own colour, so the server can colour the rest.

## What inkline does with a reply

- The spans go over bash's colours in the argument, on the characters that
  were typed for those bytes. Quote marks are never painted, nor, in a
  `final` argument, the backslashes bash removes. A variable such as `$n`
  or `${n}` keeps bash's colour.
- Every character of an argument that got at least one span is drawn with
  the `script` style on top (dim, by default), except its quote marks and,
  in a `final` argument, the backslashes bash removes.
- The error is underlined once the user pauses typing, as a bash syntax error
  is, and `NAME: MESSAGE` (the command's name as typed, after quotes are
  removed, then the message) shows under the line for as long as the underline
  does. An empty range (`START = END`) underlines the character after it, or the
  argument's last character when it is at the end. An error with no place, with
  a range outside the argument, or with `START > END` shows only its message. An
  error is not shown while the cursor is right at the end of its range (the user
  is still typing it), inside a `raw` argument (bash will change that text), or
  when bash sees a syntax error on the line.
- inkline waits at most 15 ms for a reply each time it draws the line. A
  reply that comes later is not a failure: inkline paints it when it comes,
  if the arguments are still on the line.
- inkline keeps a reply and uses it again, without asking, while the same
  arguments and directory are on the line. So a change to a file the server
  reads, such as a CSV file's header, may show only once the arguments
  change, or on the next line: each new line at the prompt asks again.

## Indenting a new line (`indent`)

A server that names `indent` on its first line (`inkline-mode 1
indent`) can also say how far in a new line goes inside an argument, such as
a script. inkline asks it when C-j, or Enter on an unfinished command, adds
a line with the cursor inside a quoted `final` argument of a command that
uses the server's mode, the server is running, `inkline-indent` is above 0,
no text is being pasted, and the line is not added from Lisp (by a Lisp
command or a hook). It never sends this request to a server that did not
name `indent`.

The request:

```
:indent ID
:cwd LEN
BYTES
:arg KIND LEN
BYTES
…one :arg for each argument, as in a colour request…
:at ARG OFFSET
:done
```

- `:cwd` and the `:arg` blocks are as in a colour request.
- `:at` is where the new line breaks: `ARG` is the argument's index, and
  `OFFSET` a byte offset in that argument's `BYTES`, with
  `0 <= OFFSET <= LEN`. The text after `OFFSET` goes to the new line.

The reply:

```
:depth NEW CURRENT
:end ID
```

- `NEW` is the nesting depth of the line that starts at `OFFSET`, the new
  line, as a plain decimal number; 0 is the argument's top level.
- `CURRENT` is the depth of the line the cursor is on, the one being
  split, as a plain decimal number, or `-` to leave that line as it is:
  `:depth 1 -` means "the new line is at depth 1; leave the cursor's line
  as it is". Send `-` for a line the user may have indented by hand, and
  a number for a line that should move, such as one that starts by
  closing a group.
- A server that cannot tell, for example because `OFFSET` is not in a part
  it indents, sends only `:end ID`.
- A `:depth` without exactly two fields, whose `NEW` is not a plain
  decimal number, whose `CURRENT` is neither a plain decimal number nor
  `-`, or that holds a number too large to read, a second `:depth`, a line
  that does not start with `:`, or `:end` with the wrong `ID` turns the
  server off (`bad reply: "LINE"`). Other lines starting with `:` are
  ignored.

What inkline does with it, with `step` the value of `inkline-indent` and
`base` the indentation of the line the command's name is on:

- A line at depth `d` starts with `base`, then `(1 + d) × step` spaces. A
  depth above 20 counts as 20.
- The new line gets `NEW`'s indentation, and the spaces and tabs around
  the cursor go. The request still holds them: `OFFSET` counts them.
- When `CURRENT` is a number, the cursor's line gets `CURRENT`'s
  indentation when the cursor is past the line's first non-blank
  character, it is not the line the argument starts on, and that
  indentation is less far in than the line's own: a line that starts by
  closing a group moves back out. A line is never moved further in. When
  `CURRENT` is `-`, the cursor's line stays as it is.
- inkline waits at most 100 ms for the answer: first for the reply to a
  request already in flight, then for the depths. A reply that comes later
  is read and dropped, and is not a failure. Without depths in time, with
  only `:end`, or in a `raw` argument (inkline does not ask then), nothing
  moves, and the new line gets the indentation of the cursor's line; when
  the cursor is on the line the argument starts on, it gets depth 0's.
- Between an empty pair of quotes, as pairing leaves them after `csvm "`,
  inkline does not ask: the new line gets depth 0's indentation, and, when
  the screen has room for both lines, the closing quote goes on a line of
  its own below it, with `base`.

For example, with `(setq inkline-indent 2)` in `/home/me/sales`, the user
has typed this, and the cursor is at the end of the second line, before the
closing quote:

```
csvm 'fn f(n) {
    rename a=n'
```

The user presses C-j. Argument 1 is 24 bytes, and the cursor is at its end.
inkline sends:

```
:indent 7
:cwd 14
/home/me/sales
:arg final 4
csvm
:arg final 24
fn f(n) {
    rename a=n
:at 1 24
:done
```

The server answers that the new line is inside the group, and that the
second line stays as the user typed it:

```
:depth 1 -
:end 7
```

The new line starts with `(1 + 1) × 2` = 4 spaces, and the second line
keeps its own 4 spaces. When the user then types `}` and presses C-j, the
server answers `:depth 0 0`: the `}` closes the group and starts its line,
so the server gives that line's depth. The `}` line moves out to 2
spaces, and the new line starts there too.

## What inkline ignores

So that later versions can add to a reply, inkline ignores, without turning
the server off:

- a `:span` with a `KIND` it does not know;
- a `:span` that overlaps a span kept before it in the same argument;
- a `:span` whose `ARG` or offsets are outside the arguments, or whose
  `START` is not below its `END`;
- any other line starting with `:` whose keyword it does not know, such as
  `:hint something`.

## What turns a mode server off

A mode server that fails is turned off until the user defines its mode again
or runs `inkline reload`. The user sees `inkline: mode MODE: off (REASON)`
under the line once they pause typing, and `inkline status` shows `mode MODE
(COMMANDS): off (REASON)`. The reasons:

- `not found`: the program is not on bash's `PATH`, or does not exist.
- `cannot run: …`: the program exists but cannot be started, with the
  system's reason.
- `not a mode server`: its first line is not `inkline-mode 1`, alone or
  followed by words.
- `bad reply: "LINE"`: this line of the reply, cut to 40 bytes, breaks the
  protocol. That is a line that does not start with `:` (an empty line
  too); `:span`, `:error`, `:depth` or `:end` with fields that do not
  parse; a second `:error` or `:depth` in one reply; or `:end` with the
  wrong `ID`.
- `bad reply: too much output`: the server wrote more than 1 MiB without
  finishing its first line or a reply.
- `bad reply: request not read`: the server stopped reading its input, and
  a request could not be written within a second.
- `exited`: the server exited, or closed its end of the socket.
- `connection lost`: a command in the shell closed inkline's end of the
  socket. inkline keeps it on the highest free descriptor below 256 (or
  below the limit on open files, when that is lower), where bash keeps its
  own.

## The rules a mode server must keep

- Write the first line as soon as it starts, before reading anything.
- Answer every request, `:indent` ones too, in order, with exactly one
  reply ending in `:end ID`.
- Flush the output after each `:end` line, and after the first line. The
  output is a socket, not a terminal, so most languages buffer it until
  told to flush; a reply stuck in the buffer never reaches inkline.
- Write nothing else: no text before the first line or between replies.
- Keep reading requests: never stop reading stdin while the shell runs.
- Exit when stdin ends.
- Never write to the terminal, and never read from it. The server has no
  terminal to use.

## A working example

[`tests/data/fake-mode-server`](../tests/data/fake-mode-server) is the small
bash mode server that inkline's own tests use. It shows the whole loop: the
first line, reading `:cwd` and each `:arg` by its length with `read -N`
(under `LC_ALL=C`, so that bash counts bytes, not characters), and writing
the `:span`, `:error` and `:end` lines. Its first argument picks what it
does, so that it can also break the protocol for tests; `words` is the plain
case, and `separator` is the same but sends `separator` for `|`. `indent`
also names `indent` and answers `:indent` requests, counting the brackets
`{`, `(`, `}` and `)`; `dash-indent` does the same, but sends `-` for the
cursor's line unless it starts with `}` or `)`. Try it by hand:

```bash
printf ':request 1\n:cwd 1\n/\n:arg final 4\ncsvm\n:arg final 16\nsort id | head 5\n:done\n' |
    tests/data/fake-mode-server separator
```

It prints `inkline-mode 1`, the five `:span` lines of the example above,
and `:end 1`. To use it in a shell, define a mode for it in `init.el`:

```elisp
(inkline-define-mode 'fake-mode '("/path/to/inkline/tests/data/fake-mode-server" "words"))
(push '("csvm" . fake-mode) inkline-command-mode-alist)
```
