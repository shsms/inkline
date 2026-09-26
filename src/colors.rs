//! The colour table: the SGR codes each kind of piece is drawn with, set by
//! `inkline-colors` (see `crate::lisp::settings`), as a list of pairs or in
//! the same format as `LS_COLORS`. A value is SGR codes or colour words
//! (`codes_for`).

use crate::lexer::Kind;

pub const DEFAULT: &str = "command=32:unknown=31:keyword=35:option=36:string=33:variable=34:operator=1:comment=2:suggestion=90:number=36:function=32:script=2";

/// The start of the syntax-error underline: a plain underline first, which
/// every terminal shows, then a wavy one in red where the terminal supports
/// those codes.
const DEFAULT_ERROR: &str = "\x1b[4m\x1b[4:3m\x1b[58:5:1m";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    /// By `Kind as usize`. `None` only for `Separator` when it is not set:
    /// a separator is then drawn with the `operator` colour.
    kinds: [Option<String>; Kind::COUNT],
    suggestion: String,
    /// The SGR codes of the `script` style, drawn on top of a part's colour;
    /// empty means no style.
    script: String,
    /// The bytes that start the syntax-error underline; empty when it is off.
    error: String,
}

impl Colors {
    /// The default colours with the valid entries of `spec` applied on top.
    pub fn parse(spec: &str) -> Colors {
        let mut colors = Colors {
            kinds: Default::default(),
            suggestion: String::new(),
            script: String::new(),
            error: DEFAULT_ERROR.to_owned(),
        };
        colors.apply(DEFAULT);
        colors.apply(spec);
        colors
    }

    fn apply(&mut self, spec: &str) {
        for entry in entries(spec) {
            let Some((name, value)) = entry.split_once('=') else {
                continue;
            };
            // The string form skips an entry it cannot read.
            let Ok(codes) = codes_for(value) else {
                continue;
            };
            if name == "error" {
                self.error = if codes.is_empty() {
                    String::new()
                } else {
                    format!("\x1b[{codes}m")
                };
                continue;
            }
            if codes.is_empty() {
                continue;
            }
            if name == "suggestion" {
                self.suggestion = codes;
            } else if name == "script" {
                self.script = codes;
            } else if let Some(kind) = kind_named(name) {
                self.kinds[kind as usize] = Some(codes);
            }
        }
    }

    pub fn sgr(&self, kind: Kind) -> &str {
        match &self.kinds[kind as usize] {
            Some(codes) => codes,
            None if kind == Kind::Separator => self.sgr(Kind::Operator),
            None => "",
        }
    }

    pub fn suggestion(&self) -> &str {
        &self.suggestion
    }

    pub fn script(&self) -> &str {
        &self.script
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    /// These colours with `set`'s on top. A separator the set leaves out
    /// keeps these colours' `separator` if they have one, and otherwise takes
    /// the `operator` colour of the result.
    pub fn layered(&self, set: &ColorSet) -> Colors {
        let mut colors = self.clone();
        for (slot, codes) in colors.kinds.iter_mut().zip(&set.kinds) {
            if codes.is_some() {
                slot.clone_from(codes);
            }
        }
        if let Some(script) = &set.script {
            colors.script.clone_from(script);
        }
        colors
    }

    /// The default colours with `entries` (name, value in codes or words)
    /// applied on top. The first entry for a name wins; an empty value means
    /// no colour, and turns the underline off for `error`.
    pub fn from_entries(entries: &[(String, String)]) -> Result<Colors, String> {
        let mut colors = Colors::default();
        let mut seen: Vec<&str> = Vec::new();
        for (name, value) in entries {
            if seen.contains(&name.as_str()) {
                continue;
            }
            seen.push(name);
            let codes = codes_for(value).map_err(|e| format!("{name}: {e}"))?;
            match name.as_str() {
                "error" => {
                    colors.error = if codes.is_empty() {
                        String::new()
                    } else {
                        format!("\x1b[{codes}m")
                    }
                }
                "suggestion" => colors.suggestion = codes,
                "script" => colors.script = codes,
                _ => match kind_named(name) {
                    Some(kind) => colors.kinds[kind as usize] = Some(codes),
                    None => return Err(format!("unknown colour name {name}")),
                },
            }
        }
        Ok(colors)
    }
}

impl Default for Colors {
    fn default() -> Colors {
        Colors::parse("")
    }
}

/// A mode's own colours (`inkline-define-mode`'s third argument): SGR codes
/// for some of the ten kinds a mode server sends and for `script`. The ones
/// left out come from `inkline-colors`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorSet {
    /// By `Kind as usize`; `Unknown` is never set.
    kinds: [Option<String>; Kind::COUNT],
    script: Option<String>,
}

impl ColorSet {
    /// A set from `entries` (name, value in codes or words). The first
    /// entry for a name wins. A name other than the ten a mode server sends
    /// and `script`, or a value that cannot be read, is an error, in a
    /// later entry for a name too.
    pub fn from_entries(entries: &[(String, String)]) -> Result<ColorSet, String> {
        let mut set = ColorSet::default();
        let mut seen: Vec<&str> = Vec::new();
        for (name, value) in entries {
            let kind = crate::mode_server::protocol::kind_named(name);
            if kind.is_none() && name != "script" {
                return Err(format!("unknown colour name {name}"));
            }
            let codes = codes_for(value).map_err(|e| format!("{name}: {e}"))?;
            if seen.contains(&name.as_str()) {
                continue;
            }
            seen.push(name);
            match kind {
                Some(kind) => set.kinds[kind as usize] = Some(codes),
                None => set.script = Some(codes),
            }
        }
        Ok(set)
    }

    /// A set from `spec`, in `LS_COLORS`'s format as `Colors::parse` reads
    /// it, except that every entry must be right.
    pub fn parse(spec: &str) -> Result<ColorSet, String> {
        let mut list = Vec::new();
        for entry in entries(spec) {
            if entry.is_empty() {
                continue;
            }
            let Some((name, value)) = entry.split_once('=') else {
                return Err(format!("{entry:?} is not NAME=VALUE"));
            };
            list.push((name.to_owned(), value.to_owned()));
        }
        ColorSet::from_entries(&list)
    }
}

/// The entries of `spec`, split at `:`. A piece without `=`, made only of the
/// characters an SGR code can hold (digits and `;`), belongs to the entry
/// before it, so values can hold sub-parameters such as `4:3` or
/// `38:2::255:0:0`. A trailing `:` left by a stray separator, not a
/// sub-parameter, is dropped from the end of each entry.
fn entries(spec: &str) -> Vec<String> {
    let mut entries: Vec<String> = Vec::new();
    for piece in spec.split(':') {
        let joins = !piece.contains('=') && piece.bytes().all(|b| b.is_ascii_digit() || b == b';');
        match entries.last_mut() {
            Some(last) if joins => {
                last.push(':');
                last.push_str(piece);
            }
            _ => entries.push(piece.to_owned()),
        }
    }
    for entry in &mut entries {
        while entry.ends_with(':') {
            entry.pop();
        }
    }
    entries
}

/// The `Kind` a colour name names: `unknown`, or one of the kinds a mode
/// server sends.
fn kind_named(name: &str) -> Option<Kind> {
    if name == "unknown" {
        Some(Kind::Unknown)
    } else {
        crate::mode_server::protocol::kind_named(name)
    }
}

/// The SGR codes for a colour value. A value made only of digits, `;` and
/// `:` is codes already (an empty one means no colour). Any other value is
/// words separated by blanks, in any case: `bold`, `dim`, `italic`,
/// `underline` or `reverse`; a colour for the text; `on` and a colour for
/// the background. The error names the word that is wrong.
pub fn codes_for(value: &str) -> Result<String, String> {
    if is_codes(value) {
        return Ok(value.to_owned());
    }
    let mut codes: Vec<String> = Vec::new();
    let mut words = value.split_whitespace();
    while let Some(word) = words.next() {
        let lower = word.to_ascii_lowercase();
        if lower == "on" {
            let Some(next) = words.next() else {
                return Err("\"on\" needs a colour after it".to_owned());
            };
            let colour = colour_named(&next.to_ascii_lowercase())
                .ok_or_else(|| format!("unknown colour {next:?} after \"on\""))?;
            codes.push(colour.background());
        } else if let Some(code) = attribute(&lower) {
            codes.push(code.to_owned());
        } else if let Some(colour) = colour_named(&lower) {
            codes.push(colour.foreground());
        } else {
            return Err(format!("unknown colour word {word:?}"));
        }
    }
    Ok(codes.join(";"))
}

/// Whether `value` is made only of the characters SGR codes hold.
fn is_codes(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
}

/// The code of an attribute word, in lower case.
fn attribute(word: &str) -> Option<&'static str> {
    Some(match word {
        "bold" => "1",
        "dim" => "2",
        "italic" => "3",
        "underline" => "4",
        "reverse" => "7",
        _ => return None,
    })
}

/// The eight basic colours, in the order of their codes.
const BASIC: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

/// A colour a word names.
enum Colour {
    /// One of `BASIC`, by its place in the list.
    Basic(u8),
    /// The bright form of one of `BASIC`.
    Bright(u8),
    /// An entry of the 256-colour palette.
    Palette(u8),
    /// Red, green and blue, for 24-bit colour.
    Rgb(u8, u8, u8),
}

impl Colour {
    fn foreground(&self) -> String {
        match *self {
            Colour::Basic(n) => (30 + n).to_string(),
            Colour::Bright(n) => (90 + n).to_string(),
            Colour::Palette(n) => format!("38;5;{n}"),
            Colour::Rgb(r, g, b) => format!("38;2;{r};{g};{b}"),
        }
    }

    fn background(&self) -> String {
        match *self {
            Colour::Basic(n) => (40 + n).to_string(),
            Colour::Bright(n) => (100 + n).to_string(),
            Colour::Palette(n) => format!("48;5;{n}"),
            Colour::Rgb(r, g, b) => format!("48;2;{r};{g};{b}"),
        }
    }
}

/// The colour `word`, in lower case, names: a basic colour, its `bright-`
/// form, `grey` (bright black), `grey0` to `grey23` (the palette's greys,
/// 232 to 255), `colorN` or `colourN` (palette entry N), or `#rrggbb`.
/// `gray` is the same as `grey`.
fn colour_named(word: &str) -> Option<Colour> {
    let basic = |name: &str| {
        BASIC
            .iter()
            .position(|b| *b == name)
            .and_then(|i| u8::try_from(i).ok())
    };
    if let Some(i) = basic(word) {
        return Some(Colour::Basic(i));
    }
    if let Some(i) = word.strip_prefix("bright-").and_then(basic) {
        return Some(Colour::Bright(i));
    }
    if let Some(step) = word
        .strip_prefix("grey")
        .or_else(|| word.strip_prefix("gray"))
    {
        if step.is_empty() {
            return Some(Colour::Bright(0));
        }
        return number(step)
            .filter(|&n| n <= 23)
            .map(|n| Colour::Palette(232 + n));
    }
    if let Some(n) = word
        .strip_prefix("colour")
        .or_else(|| word.strip_prefix("color"))
    {
        return number(n).map(Colour::Palette);
    }
    if let Some(hex) = word.strip_prefix('#') {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
        return Some(Colour::Rgb(byte(0)?, byte(2)?, byte(4)?));
    }
    None
}

/// A number from 0 to 255 written only in decimal digits.
fn number(digits: &str) -> Option<u8> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn entries_first_wins_and_empty_means_plain() {
        let colors = Colors::from_entries(&entries(&[
            ("command", "35"),
            ("command", "36"),
            ("string", ""),
            ("error", ""),
            ("suggestion", "38:5:244"),
        ]))
        .unwrap();
        assert_eq!(colors.sgr(Kind::Command), "35");
        assert_eq!(colors.sgr(Kind::String), "");
        assert_eq!(colors.error(), "");
        assert_eq!(colors.suggestion(), "38:5:244");
        assert_eq!(colors.sgr(Kind::Keyword), "35");
    }

    #[test]
    fn entries_reject_unknown_names_and_bad_codes() {
        assert_eq!(
            Colors::from_entries(&entries(&[("comand", "1")])),
            Err("unknown colour name comand".to_owned())
        );
        assert_eq!(
            Colors::from_entries(&entries(&[("command", "redd")])),
            Err("command: unknown colour word \"redd\"".to_owned())
        );
    }

    #[test]
    fn defaults() {
        let colors = Colors::default();
        assert_eq!(colors.sgr(Kind::Command), "32");
        assert_eq!(colors.sgr(Kind::Unknown), "31");
        assert_eq!(colors.sgr(Kind::Comment), "2");
        assert_eq!(colors.sgr(Kind::Number), "36");
        assert_eq!(colors.sgr(Kind::Function), "32");
        assert_eq!(colors.suggestion(), "90");
        assert_eq!(colors.script(), "2");
    }

    #[test]
    fn script_style_from_entries() {
        let colors = Colors::from_entries(&entries(&[("script", "48;5;236")])).unwrap();
        assert_eq!(colors.script(), "48;5;236");
        let colors = Colors::from_entries(&entries(&[("script", "")])).unwrap();
        assert_eq!(colors.script(), "");
    }

    #[test]
    fn entries_override_only_their_kind() {
        let colors = Colors::parse("command=1;35:suggestion=38;5;244");
        assert_eq!(colors.sgr(Kind::Command), "1;35");
        assert_eq!(colors.sgr(Kind::String), "33");
        assert_eq!(colors.suggestion(), "38;5;244");
    }

    #[test]
    fn invalid_entries_are_ignored() {
        let colors = Colors::parse("command=redd:bogus=1:string=:=4:novalue:operator=38;5;208");
        assert_eq!(colors.sgr(Kind::Command), "32");
        assert_eq!(colors.sgr(Kind::String), "33");
        assert_eq!(colors.sgr(Kind::Operator), "38;5;208");
    }

    #[test]
    fn error_underline_default_and_overrides() {
        assert_eq!(Colors::default().error(), "\x1b[4m\x1b[4:3m\x1b[58:5:1m");
        assert_eq!(Colors::parse("error=4").error(), "\x1b[4m");
        assert_eq!(
            Colors::parse("error=4:3;58:5:1").error(),
            "\x1b[4:3;58:5:1m"
        );
        assert_eq!(Colors::parse("error=").error(), "");
        assert_eq!(
            Colors::parse("error=wavy").error(),
            Colors::default().error()
        );
    }

    #[test]
    fn values_can_hold_sub_parameters() {
        let colors = Colors::parse("command=38:5:208:string=1");
        assert_eq!(colors.sgr(Kind::Command), "38:5:208");
        assert_eq!(colors.sgr(Kind::String), "1");
    }

    #[test]
    fn a_trailing_colon_does_not_change_the_value() {
        assert_eq!(Colors::parse("command=32:").sgr(Kind::Command), "32");
    }

    #[test]
    fn an_empty_field_between_entries_is_not_a_sub_parameter() {
        let colors = Colors::parse("command=32::string=33");
        assert_eq!(colors.sgr(Kind::Command), "32");
        assert_eq!(colors.sgr(Kind::String), "33");
    }

    #[test]
    fn a_nameless_word_does_not_join_the_entry_before_it() {
        let colors = Colors::parse("command=32:novalue:string=33");
        assert_eq!(colors.sgr(Kind::Command), "32");
        assert_eq!(colors.sgr(Kind::String), "33");
    }

    #[test]
    fn colour_words() {
        for (value, codes) in [
            ("bold magenta", "1;35"),
            ("dim", "2"),
            ("italic underline reverse", "3;4;7"),
            ("on grey4", "48;5;236"),
            ("underline bright-cyan on black", "4;96;40"),
            ("bright-red on bright-blue", "91;104"),
            ("white", "37"),
            ("bright-white", "97"),
            ("bright-black", "90"),
            ("grey", "90"),
            ("gray", "90"),
            ("on grey", "100"),
            ("grey0", "38;5;232"),
            ("grey23", "38;5;255"),
            ("GRAY23", "38;5;255"),
            ("color0", "38;5;0"),
            ("colour255", "38;5;255"),
            ("on color17", "48;5;17"),
            ("#3a3a3a", "38;2;58;58;58"),
            ("on #FF0080", "48;2;255;0;128"),
            ("Bold  RED", "1;31"),
            ("red blue", "31;34"),
        ] {
            assert_eq!(codes_for(value), Ok(codes.to_owned()), "{value}");
        }
    }

    #[test]
    fn plain_codes_are_kept() {
        for value in ["38;5;208", "4:3", "1", ""] {
            assert_eq!(codes_for(value), Ok(value.to_owned()), "{value}");
        }
    }

    #[test]
    fn a_wrong_word_is_named() {
        for (value, error) in [
            ("magneta", "unknown colour word \"magneta\""),
            ("bold Magneta", "unknown colour word \"Magneta\""),
            ("bold on", "\"on\" needs a colour after it"),
            ("on bold", "unknown colour \"bold\" after \"on\""),
            ("grey24", "unknown colour word \"grey24\""),
            ("colour256", "unknown colour word \"colour256\""),
            ("color", "unknown colour word \"color\""),
            ("#12345", "unknown colour word \"#12345\""),
            ("#gggggg", "unknown colour word \"#gggggg\""),
            ("bright-grey", "unknown colour word \"bright-grey\""),
            ("grey+1", "unknown colour word \"grey+1\""),
        ] {
            assert_eq!(codes_for(value), Err(error.to_owned()), "{value}");
        }
    }

    #[test]
    fn values_in_words() {
        let colors =
            Colors::from_entries(&entries(&[("command", "red"), ("script", "on grey3")])).unwrap();
        assert_eq!(colors.sgr(Kind::Command), "31");
        assert_eq!(colors.script(), "48;5;235");
        assert_eq!(
            Colors::from_entries(&entries(&[("error", "underline")]))
                .unwrap()
                .error(),
            "\x1b[4m"
        );
        let colors = Colors::parse("command=bold magenta:script=on grey3");
        assert_eq!(colors.sgr(Kind::Command), "1;35");
        assert_eq!(colors.script(), "48;5;235");
    }

    #[test]
    fn a_command_set_goes_over_the_colours() {
        let base = Colors::parse("variable=34:script=2");
        let set = ColorSet::from_entries(&entries(&[
            ("command", "bold magenta"),
            ("script", "on grey3"),
            ("command", "1"),
        ]))
        .unwrap();
        let colors = base.layered(&set);
        assert_eq!(colors.sgr(Kind::Command), "1;35", "the first entry wins");
        assert_eq!(colors.script(), "48;5;235");
        assert_eq!(
            colors.sgr(Kind::Variable),
            "34",
            "left out: the colours below"
        );
        assert_eq!(colors.suggestion(), base.suggestion());
        assert_eq!(colors.error(), base.error());
        let off = ColorSet::from_entries(&entries(&[("script", "")])).unwrap();
        assert_eq!(
            base.layered(&off).script(),
            "",
            "an empty value is no style"
        );
    }

    #[test]
    fn a_mode_set_takes_only_the_server_names() {
        for name in [
            "command",
            "keyword",
            "option",
            "operator",
            "string",
            "number",
            "variable",
            "function",
            "comment",
            "separator",
            "script",
        ] {
            assert!(
                ColorSet::from_entries(&entries(&[(name, "1")])).is_ok(),
                "{name}"
            );
        }
        for name in ["unknown", "suggestion", "error", "comand"] {
            assert_eq!(
                ColorSet::from_entries(&entries(&[(name, "1")])),
                Err(format!("unknown colour name {name}"))
            );
        }
        assert_eq!(
            ColorSet::from_entries(&entries(&[("number", "on")])),
            Err("number: \"on\" needs a colour after it".to_owned())
        );
    }

    #[test]
    fn a_separator_takes_the_operator_colour_until_it_is_set() {
        assert_eq!(Colors::default().sgr(Kind::Separator), "1");
        let colors = Colors::parse("operator=31");
        assert_eq!(colors.sgr(Kind::Separator), "31");
        let colors = Colors::parse("operator=31:separator=bold cyan");
        assert_eq!(colors.sgr(Kind::Separator), "1;36");
        assert_eq!(colors.sgr(Kind::Operator), "31");
        let colors = Colors::from_entries(&entries(&[("operator", "31")])).unwrap();
        assert_eq!(colors.sgr(Kind::Separator), "31");
        let colors =
            Colors::from_entries(&entries(&[("operator", "31"), ("separator", "")])).unwrap();
        assert_eq!(colors.sgr(Kind::Separator), "", "an empty value is plain");
    }

    /// A mode's `separator`, else `inkline-colors`' `separator`, else the
    /// `operator` colour found the same way.
    #[test]
    fn a_separator_colour_is_looked_up_in_order() {
        let set = |spec| ColorSet::parse(spec).unwrap();
        let plain = Colors::parse("operator=31");
        let own = Colors::parse("operator=31:separator=32");
        assert_eq!(own.layered(&set("separator=33")).sgr(Kind::Separator), "33");
        assert_eq!(own.layered(&set("operator=34")).sgr(Kind::Separator), "32");
        assert_eq!(
            plain.layered(&set("operator=34")).sgr(Kind::Separator),
            "34"
        );
        assert_eq!(plain.layered(&set("")).sgr(Kind::Separator), "31");
        assert_eq!(
            plain.layered(&set("separator=33")).sgr(Kind::Operator),
            "31"
        );
    }

    #[test]
    fn a_command_set_string_must_be_right() {
        let set = ColorSet::parse("command=bold magenta:number=38:5:208").unwrap();
        let colors = Colors::default().layered(&set);
        assert_eq!(
            (colors.sgr(Kind::Command), colors.sgr(Kind::Number)),
            ("1;35", "38:5:208")
        );
        assert_eq!(ColorSet::parse(""), Ok(ColorSet::default()));
        assert_eq!(
            ColorSet::parse("command=magneta"),
            Err("command: unknown colour word \"magneta\"".to_owned())
        );
        assert_eq!(
            ColorSet::parse("bogus=1"),
            Err("unknown colour name bogus".to_owned())
        );
        assert_eq!(
            ColorSet::parse("command"),
            Err("\"command\" is not NAME=VALUE".to_owned())
        );
    }

    /// The first entry for a name wins, but a later one is checked too.
    #[test]
    fn a_command_sets_later_entries_must_be_right() {
        let set = ColorSet::parse("command=31:command=32").unwrap();
        assert_eq!(Colors::default().layered(&set).sgr(Kind::Command), "31");
        assert_eq!(
            ColorSet::parse("command=31:command=magneta"),
            Err("command: unknown colour word \"magneta\"".to_owned())
        );
        assert_eq!(
            ColorSet::from_entries(&entries(&[("number", "1"), ("number", "on")])),
            Err("number: \"on\" needs a colour after it".to_owned())
        );
    }
}
