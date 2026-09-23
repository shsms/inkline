//! Decides whether a command word names something bash can run.

use std::collections::HashSet;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::SystemTime;

/// The executable names in the directories of `PATH`. Rebuilt when `PATH`
/// changes or one of its directories is modified. A name the cache lacks is
/// looked up directly, because a directory's timestamp can miss a program
/// installed within its resolution (a second on some filesystems).
#[derive(Default)]
pub struct PathCache {
    path: String,
    stamps: Vec<Option<SystemTime>>,
    names: HashSet<String>,
}

impl PathCache {
    pub fn contains(&mut self, name: &str, path: &str) -> bool {
        let stamps = dir_stamps(path);
        if path != self.path || stamps != self.stamps {
            self.rebuild(path, stamps);
        }
        self.names.contains(name)
            || path_dirs(path).any(|dir| is_executable(&Path::new(dir).join(name)))
    }

    fn rebuild(&mut self, path: &str, stamps: Vec<Option<SystemTime>>) {
        self.names.clear();
        for dir in path_dirs(path) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if is_executable(&entry.path())
                    && let Ok(name) = entry.file_name().into_string()
                {
                    self.names.insert(name);
                }
            }
        }
        self.path = path.to_string();
        self.stamps = stamps;
    }
}

/// The directories of `path`. bash reads an empty entry as the current
/// directory; the cache leaves those out.
fn path_dirs(path: &str) -> impl Iterator<Item = &str> {
    path.split(':').filter(|d| !d.is_empty())
}

fn dir_stamps(path: &str) -> Vec<Option<SystemTime>> {
    path_dirs(path)
        .map(|d| std::fs::metadata(d).and_then(|m| m.modified()).ok())
        .collect()
}

fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// Whether bash can run `word`. `known_to_bash` answers for keywords, aliases,
/// functions and builtins; a word with a `/` is checked as a path.
pub fn exists(
    word: &str,
    path: &str,
    cache: &mut PathCache,
    known_to_bash: impl Fn(&str) -> bool,
) -> bool {
    if word.contains('/') {
        return is_executable(Path::new(word));
    }
    known_to_bash(word) || cache.contains(word, path)
}

/// Whether `word` can be looked up as written: no expansions, quotes or globs,
/// which only bash can resolve.
pub fn is_plain(word: &str) -> bool {
    !word.is_empty() && !word.contains(['$', '`', '"', '\'', '\\', '*', '?', '[', '~', '{'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    fn file(dir: &Path, name: &str, mode: u32) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn finds_executables_on_path() {
        let dir = tempfile::tempdir().unwrap();
        file(dir.path(), "tool", 0o755);
        file(dir.path(), "data", 0o644);
        let path = format!("/nonexistent:{}", dir.path().display());
        let mut cache = PathCache::default();
        assert!(cache.contains("tool", &path));
        assert!(!cache.contains("data", &path));
        assert!(!cache.contains("missing", &path));
    }

    #[test]
    fn notices_new_programs_and_path_changes() {
        let one = tempfile::tempdir().unwrap();
        let two = tempfile::tempdir().unwrap();
        file(two.path(), "other", 0o755);
        let path = one.path().display().to_string();
        let mut cache = PathCache::default();
        assert!(!cache.contains("later", &path));
        file(one.path(), "later", 0o755);
        assert!(cache.contains("later", &path));
        assert!(!cache.contains("other", &path));
        assert!(cache.contains("other", &two.path().display().to_string()));
    }

    #[test]
    fn asks_bash_first_and_checks_paths_directly() {
        let dir = tempfile::tempdir().unwrap();
        let script = file(dir.path(), "script", 0o755);
        let mut cache = PathCache::default();
        assert!(exists("cd", "", &mut cache, |w| w == "cd"));
        assert!(!exists("nope", "", &mut cache, |_| false));
        assert!(exists(script.to_str().unwrap(), "", &mut cache, |_| false));
        assert!(!exists("/no/such/file", "", &mut cache, |_| true));
    }

    #[test]
    fn plain_words() {
        assert!(is_plain("ls"));
        assert!(is_plain("./run.sh"));
        assert!(!is_plain("$cmd"));
        assert!(!is_plain("\"ls\""));
        assert!(!is_plain("~/bin/x"));
        assert!(!is_plain(""));
    }
}
