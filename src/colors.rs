//! The colour table: the SGR codes each kind of piece is drawn with, set by the
//! `INKLINE_COLORS` shell variable in the same format as `LS_COLORS`.

use crate::lexer::Kind;

pub const DEFAULT: &str = "command=32:unknown=31:keyword=35:option=36:string=33:variable=34:operator=1:comment=2:suggestion=90";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Colors {
    kinds: [String; 8],
    suggestion: String,
}

impl Colors {
    /// The default colours with the valid entries of `spec` applied on top.
    pub fn parse(spec: &str) -> Colors {
        let mut colors = Colors {
            kinds: Default::default(),
            suggestion: String::new(),
        };
        colors.apply(DEFAULT);
        colors.apply(spec);
        colors
    }

    fn apply(&mut self, spec: &str) {
        for entry in spec.split(':') {
            let Some((name, codes)) = entry.split_once('=') else {
                continue;
            };
            if codes.is_empty() || !codes.bytes().all(|b| b.is_ascii_digit() || b == b';') {
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
}

impl Default for Colors {
    fn default() -> Colors {
        Colors::parse("")
    }
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
}
