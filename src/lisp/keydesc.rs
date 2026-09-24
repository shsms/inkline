//! Emacs key descriptions (`C-x C-e`, `M-RET`, `<up>`) as the byte sequences
//! a terminal sends.

/// The sequences terminals send for each named key.
const NAMED: &[(&str, &[&[u8]])] = &[
    ("<up>", &[b"\x1b[A", b"\x1bOA"]),
    ("<down>", &[b"\x1b[B", b"\x1bOB"]),
    ("<right>", &[b"\x1b[C", b"\x1bOC"]),
    ("<left>", &[b"\x1b[D", b"\x1bOD"]),
    ("<home>", &[b"\x1b[H", b"\x1bOH", b"\x1b[1~", b"\x1b[7~"]),
    ("<end>", &[b"\x1b[F", b"\x1bOF", b"\x1b[4~", b"\x1b[8~"]),
    ("<delete>", &[b"\x1b[3~"]),
    ("<backtab>", &[b"\x1b[Z"]),
];

/// The most byte sequences one description may stand for.
const MAX_SEQS: usize = 16;

/// Every byte sequence `desc` stands for: one, or one per sequence of a named
/// key in it; at most `MAX_SEQS`.
pub fn parse(desc: &str) -> Result<Vec<Vec<u8>>, String> {
    let mut seqs: Vec<Vec<u8>> = vec![Vec::new()];
    let mut any = false;
    for key in desc.split_whitespace() {
        any = true;
        let options = key_options(key)?;
        seqs = seqs
            .iter()
            .flat_map(|prefix| {
                options.iter().map(move |o| {
                    let mut s = prefix.clone();
                    s.extend_from_slice(o);
                    s
                })
            })
            .collect();
        if seqs.len() > MAX_SEQS {
            return Err(format!(
                "{desc}: stands for more than {MAX_SEQS} key sequences"
            ));
        }
    }
    if !any {
        return Err("empty key description".into());
    }
    Ok(seqs)
}

fn key_options(key: &str) -> Result<Vec<Vec<u8>>, String> {
    if let Some((_, seqs)) = NAMED.iter().find(|(name, _)| *name == key) {
        return Ok(seqs.iter().map(|s| s.to_vec()).collect());
    }
    if key.starts_with('<') && key.ends_with('>') && key.len() > 2 {
        return Err(format!("{key}: unknown key"));
    }
    let (mut ctrl, mut meta, mut rest) = (false, false, key);
    loop {
        if rest.len() > 2
            && let Some(r) = rest.strip_prefix("C-")
        {
            ctrl = true;
            rest = r;
        } else if rest.len() > 2
            && let Some(r) = rest.strip_prefix("M-")
        {
            meta = true;
            rest = r;
        } else {
            break;
        }
    }
    if NAMED.iter().any(|(name, _)| *name == rest) {
        return Err(format!("{key}: modifiers on {rest} are not supported"));
    }
    let mut bytes = match rest {
        "RET" => vec![b'\r'],
        "TAB" => vec![b'\t'],
        "DEL" => vec![0x7f],
        "SPC" => vec![b' '],
        "ESC" => vec![0x1b],
        _ => {
            let mut chars = rest.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(format!(
                    "{key}: one key at a time; separate keys with spaces"
                ));
            };
            c.to_string().into_bytes()
        }
    };
    if ctrl {
        let [b] = bytes[..] else {
            return Err(format!("{key}: no control character for {rest}"));
        };
        bytes = vec![match b.to_ascii_uppercase() {
            b' ' | b'@' => 0,
            b'?' => 0x7f,
            c @ b'A'..=b'_' => c & 0x1f,
            _ => return Err(format!("{key}: no control character for {rest}")),
        }];
    }
    if meta {
        bytes.insert(0, 0x1b);
    }
    Ok(vec![bytes])
}

/// `seq` in readline's key sequence text, as `bind` takes it.
pub fn readline_text(seq: &[u8]) -> String {
    let mut out = String::new();
    for &b in seq {
        match b {
            0x1b => out.push_str(r"\e"),
            0x7f => out.push_str(r"\C-?"),
            b'\\' => out.push_str(r"\\"),
            b'"' => out.push_str("\\\""),
            // In octal: readline versions differ on `\C-\` followed by another
            // key, and on `\C-\\`.
            0x1c => out.push_str(r"\034"),
            0..=0x1f => {
                out.push_str(r"\C-");
                out.push(((b | 0x40) as char).to_ascii_lowercase());
            }
            0x80.. => out.push_str(&format!("\\{b:03o}")),
            0x20..=0x7e => out.push(b as char),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(desc: &str) -> Vec<u8> {
        let mut seqs = parse(desc).unwrap();
        assert_eq!(seqs.len(), 1, "{desc}");
        seqs.remove(0)
    }

    #[test]
    fn plain_and_modified_keys() {
        assert_eq!(one("a"), b"a");
        assert_eq!(one("C-a"), b"\x01");
        assert_eq!(one("C-A"), b"\x01");
        assert_eq!(one("M-f"), b"\x1bf");
        assert_eq!(one("C-M-x"), b"\x1b\x18");
        assert_eq!(one("M-C-x"), b"\x1b\x18");
        assert_eq!(one("C-x C-e"), b"\x18\x05");
        assert_eq!(one("RET"), b"\r");
        assert_eq!(one("M-RET"), b"\x1b\r");
        assert_eq!(one("TAB"), b"\t");
        assert_eq!(one("DEL"), b"\x7f");
        assert_eq!(one("SPC"), b" ");
        assert_eq!(one("ESC"), b"\x1b");
        assert_eq!(one("C-SPC"), b"\x00");
        assert_eq!(one("C-@"), b"\x00");
        assert_eq!(one("C-?"), b"\x7f");
        assert_eq!(one("M-#"), b"\x1b#");
        assert_eq!(one("("), b"(");
        assert_eq!(one("é"), "é".as_bytes());
    }

    #[test]
    fn named_keys_stand_for_every_sequence() {
        assert_eq!(
            parse("<up>").unwrap(),
            vec![b"\x1b[A".to_vec(), b"\x1bOA".to_vec()]
        );
        assert_eq!(parse("<home>").unwrap().len(), 4);
        assert_eq!(
            parse("C-x <up>").unwrap(),
            vec![b"\x18\x1b[A".to_vec(), b"\x18\x1bOA".to_vec()]
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse(""), Err("empty key description".into()));
        assert_eq!(
            parse("M-<up>"),
            Err("M-<up>: modifiers on <up> are not supported".into())
        );
        assert_eq!(
            parse("C-<up>"),
            Err("C-<up>: modifiers on <up> are not supported".into())
        );
        assert_eq!(parse("<pgup>"), Err("<pgup>: unknown key".into()));
        assert_eq!(parse("<home> <end>").map(|s| s.len()), Ok(16));
        assert_eq!(
            parse("<home> <end> <up>"),
            Err("<home> <end> <up>: stands for more than 16 key sequences".into())
        );
        assert_eq!(parse("C-1"), Err("C-1: no control character for 1".into()));
        assert_eq!(
            parse("ab"),
            Err("ab: one key at a time; separate keys with spaces".into())
        );
    }

    #[test]
    fn readline_text_form() {
        assert_eq!(readline_text(b"\x1b[A"), r"\e[A");
        assert_eq!(readline_text(b"\x18\x05"), r"\C-x\C-e");
        assert_eq!(readline_text(b"\x00"), r"\C-@");
        assert_eq!(readline_text(b"\x7f"), r"\C-?");
        assert_eq!(readline_text(b"\\\""), r#"\\\""#);
        assert_eq!(readline_text(b"\x1c"), r"\034");
        assert_eq!(readline_text(b"\x1ce"), r"\034e");
        assert_eq!(readline_text("é".as_bytes()), r"\303\251");
    }
}
