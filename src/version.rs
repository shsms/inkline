//! Reading bash's version, which inkline checks when `enable -f` loads it,
//! before it hooks into readline.

/// The major and minor version in `version`, such as (5, 3) for "5.3".
pub fn major_minor(version: &str) -> Option<(u32, u32)> {
    let (major, minor) = version.split_once('.')?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn major_minor_reads_dist_version() {
        assert_eq!(major_minor("5.3"), Some((5, 3)));
        assert_eq!(major_minor("5.10"), Some((5, 10)));
        assert_eq!(major_minor("6"), None);
        assert_eq!(major_minor("5.3-rc1"), None);
        assert_eq!(major_minor(""), None);
    }
}
