//! The colour table: the SGR codes each kind of piece is drawn with, set by
//! `inkline-colors` (see `crate::lisp::settings`), as a list of pairs or in
//! the same format as `LS_COLORS`.

use crate::lexer::Kind;

pub const DEFAULT: &str = "command=32:unknown=31:keyword=35:option=36:string=33:variable=34:operator=1:comment=2:suggestion=90";

/// The start of the syntax-error underline: a plain underline first, which
/// every terminal shows, then a wavy one in red where the terminal supports
/// those codes.
const DEFAULT_ERROR: &str = "\x1b[4m\x1b[4:3m\x1b[58:5:1m";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    kinds: [String; 10],
    suggestion: String,
    /// The bytes that start the syntax-error underline; empty when it is off.
    error: String,
}

impl Colors {
    /// The default colours with the valid entries of `spec` applied on top.
    pub fn parse(spec: &str) -> Colors {
        let mut colors = Colors {
            kinds: Default::default(),
            suggestion: String::new(),
            error: DEFAULT_ERROR.to_owned(),
        };
        colors.apply(DEFAULT);
        colors.apply(spec);
        colors
    }

    fn apply(&mut self, spec: &str) {
        for entry in entries(spec) {
            let Some((name, codes)) = entry.split_once('=') else {
                continue;
            };
            if !codes
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
            {
                continue;
            }
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
            let slot = if name == "suggestion" {
                &mut self.suggestion
            } else if let Some(kind) = kind_named(name) {
                &mut self.kinds[kind as usize]
            } else {
                continue;
            };
            *slot = codes.to_string();
        }
    }

    pub fn sgr(&self, kind: Kind) -> &str {
        &self.kinds[kind as usize]
    }

    pub fn suggestion(&self) -> &str {
        &self.suggestion
    }

    pub fn error(&self) -> &str {
        &self.error
    }

    /// The default colours with `entries` (name, SGR codes) applied on top.
    /// The first entry for a name wins; empty codes mean no colour, and turn
    /// the underline off for `error`.
    pub fn from_entries(entries: &[(String, String)]) -> Result<Colors, String> {
        let mut colors = Colors::default();
        let mut seen: Vec<&str> = Vec::new();
        for (name, codes) in entries {
            if seen.contains(&name.as_str()) {
                continue;
            }
            seen.push(name);
            if !codes
                .bytes()
                .all(|b| b.is_ascii_digit() || b == b';' || b == b':')
            {
                return Err(format!("{name}: {codes:?} is not an SGR code"));
            }
            match name.as_str() {
                "error" => {
                    colors.error = if codes.is_empty() {
                        String::new()
                    } else {
                        format!("\x1b[{codes}m")
                    }
                }
                "suggestion" => colors.suggestion = codes.clone(),
                _ => match kind_named(name) {
                    Some(kind) => colors.kinds[kind as usize] = codes.clone(),
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

fn kind_named(name: &str) -> Option<Kind> {
    Some(match name {
        "command" => Kind::Command,
        "unknown" => Kind::Unknown,
        "keyword" => Kind::Keyword,
        "option" => Kind::Option,
        "string" => Kind::String,
        "variable" => Kind::Variable,
        "operator" => Kind::Operator,
        "comment" => Kind::Comment,
        _ => return None,
    })
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
            Colors::from_entries(&entries(&[("command", "red")])),
            Err("command: \"red\" is not an SGR code".to_owned())
        );
    }

    #[test]
    fn defaults() {
        let colors = Colors::default();
        assert_eq!(colors.sgr(Kind::Command), "32");
        assert_eq!(colors.sgr(Kind::Unknown), "31");
        assert_eq!(colors.sgr(Kind::Comment), "2");
        assert_eq!(colors.suggestion(), "90");
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
        let colors = Colors::parse("command=red:bogus=1:string=:=4:novalue:operator=38;5;208");
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
}
