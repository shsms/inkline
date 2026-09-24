//! inkline's default key layout.

/// The default layout's groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Suggestions,
    MultiLine,
    Pairing,
}

impl Group {
    pub const ALL: [Group; 3] = [Group::Suggestions, Group::MultiLine, Group::Pairing];

    pub fn name(self) -> &'static str {
        match self {
            Group::Suggestions => "suggestions",
            Group::MultiLine => "multi-line",
            Group::Pairing => "pairing",
        }
    }

    pub fn named(name: &str) -> Option<Group> {
        Group::ALL.into_iter().find(|g| g.name() == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_are_found_by_name() {
        for g in Group::ALL {
            assert_eq!(Group::named(g.name()), Some(g));
        }
        assert_eq!(Group::MultiLine.name(), "multi-line");
        assert_eq!(Group::named("multiline"), None);
    }
}
