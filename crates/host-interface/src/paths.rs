//! How a Host writes its paths (`runner.md` § Windows paths): the backend
//! builds paths on a Host, such as where an attachment is saved, with the
//! separator of the system the Host's runner reports, so a Windows Host
//! shows `C:\Users\alex\.demi\attachments\…`, never `C:\Users\alex/.demi/…`.
//! Every path the backend joins, splits or walks up on a Host goes through
//! here.

/// The path form of a Host's system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPaths {
    /// Linux and macOS: `/` separates, and `/` is the root.
    #[default]
    Posix,
    /// Windows: `\` separates, though `/` is read as one too, and a path
    /// starts at a drive, `C:\`, or a share, `\\server\share\`.
    Windows,
}

impl HostPaths {
    /// The separator the Host writes.
    pub fn separator(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }

    /// Whether `c` separates components on the Host.
    fn separates(self, c: char) -> bool {
        c == '/' || (self == Self::Windows && c == '\\')
    }

    /// The path of `name`, one component or a relative path written with
    /// `/`, in `directory`, with the Host's separator throughout what is
    /// added.
    pub fn join(self, directory: &str, name: &str) -> String {
        let directory = directory.trim_end_matches(|c| self.separates(c));
        let name = match self {
            Self::Posix => name.to_owned(),
            Self::Windows => name.replace('/', "\\"),
        };
        format!("{directory}{}{name}", self.separator())
    }

    /// `path` read against `directory`, as a shell in `directory` reads it:
    /// as it is when it is absolute, otherwise under `directory`, with `.`
    /// and `..` taken out, in the Host's spelling of paths.
    pub fn resolve(self, directory: &str, path: &str) -> String {
        let style = match self {
            Self::Posix => typed_path::PathType::Unix,
            Self::Windows => typed_path::PathType::Windows,
        };
        typed_path::Utf8TypedPath::new(directory, style)
            .join(path)
            .normalize()
            .into_string()
    }

    /// `path` read against `directory` as the Host's own path join reads
    /// it, without taking out `.` and `..`, so the Host, which follows
    /// links first, decides where they lead: an absolute path as it is; on
    /// Windows a rooted one, `\rules\review.yaml`, on `directory`'s drive,
    /// and a drive-relative one, `C:foo`, as it is, for the Host to read
    /// against that drive's directory; otherwise joined to `directory`.
    pub fn beside(self, directory: &str, path: &str) -> String {
        match self {
            Self::Posix if path.starts_with('/') => path.to_owned(),
            Self::Posix => self.join(directory, path),
            Self::Windows => {
                use typed_path::{Utf8WindowsComponent, Utf8WindowsPath};
                let prefix = |path: &str| match Utf8WindowsPath::new(path).components().next() {
                    Some(Utf8WindowsComponent::Prefix(prefix)) => Some(prefix.as_str().to_owned()),
                    _ => None,
                };
                if prefix(path).is_some() {
                    // A drive or a share of its own, `C:foo` included,
                    // which the Host reads against that drive.
                    path.to_owned()
                } else if path.starts_with(['\\', '/']) {
                    // Rooted on the directory's drive.
                    format!("{}{path}", prefix(directory).unwrap_or_default())
                } else {
                    self.join(directory, path)
                }
            }
        }
    }

    /// The last component of `path`: the name of what it names.
    pub fn file_name(self, path: &str) -> &str {
        let path = path.trim_end_matches(|c| self.separates(c));
        path.rsplit(|c| self.separates(c)).next().unwrap_or(path)
    }

    /// `path` and every directory above it, nearest first, up to its root:
    /// `/` on a Posix Host, the drive, `C:\`, or the share on Windows. A
    /// relative path walks up to its first component.
    pub fn ancestors(self, path: &str) -> Vec<String> {
        let root = self.root(path);
        let mut directory = path.trim_end_matches(|c| self.separates(c)).to_owned();
        if directory.len() < root.len() {
            return vec![root];
        }
        let mut ancestors = vec![directory.clone()];
        while let Some(end) = directory.rfind(|c| self.separates(c)) {
            directory.truncate(end);
            if directory.len() < root.len() {
                break;
            }
            ancestors.push(directory.clone());
        }
        if !root.is_empty() {
            ancestors.push(root);
        }
        ancestors
    }

    /// The root `path` starts at, with its separator: `/`, `C:\` or
    /// `\\server\share\`; empty for a relative path.
    fn root(self, path: &str) -> String {
        match self {
            Self::Posix if path.starts_with('/') => "/".to_owned(),
            Self::Posix => String::new(),
            Self::Windows => {
                let bytes = path.as_bytes();
                if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
                    return format!("{}\\", &path[..2]);
                }
                let share = path.strip_prefix("\\\\").or_else(|| path.strip_prefix("//"));
                if let Some(share) = share {
                    let mut parts = share.splitn(3, |c| self.separates(c));
                    if let (Some(server), Some(name)) = (parts.next(), parts.next()) {
                        return format!("\\\\{server}\\{name}\\");
                    }
                }
                String::new()
            }
        }
    }
}

/// `path` relative to `directory` when it is inside it, `.` for
/// `directory` itself; none when it is outside. Each path is read in the
/// form its text has, a Windows one by its drive or share.
pub fn relative_to(directory: &str, path: &str) -> Option<String> {
    let path = typed_path::Utf8TypedPath::derive(path).normalize();
    let directory = typed_path::Utf8TypedPath::derive(directory).normalize();
    let relative = path.strip_prefix(&directory).ok()?.to_string();
    Some(if relative.is_empty() { ".".to_owned() } else { relative })
}

#[cfg(test)]
mod tests {
    use super::HostPaths::{Posix, Windows};

    #[test]
    fn a_name_is_joined_with_the_hosts_separator() {
        let cases = [
            (Posix, "/home/alex", ".demi", "/home/alex/.demi"),
            (Posix, "/home/alex/", ".demi/attachments", "/home/alex/.demi/attachments"),
            (Posix, "/", "etc", "/etc"),
            (Windows, r"C:\Users\alex", ".demi", r"C:\Users\alex\.demi"),
            (Windows, r"C:\Users\alex\", ".demi/attachments", r"C:\Users\alex\.demi\attachments"),
            (Windows, r"C:\", "Users", r"C:\Users"),
        ];
        for (paths, directory, name, joined) in cases {
            assert_eq!(paths.join(directory, name), joined, "{paths:?} {directory} {name}");
        }
    }

    #[test]
    fn a_paths_name_is_its_last_component() {
        assert_eq!(Posix.file_name("/home/alex/skills/review"), "review");
        assert_eq!(Posix.file_name("/home/alex/skills/review/"), "review");
        assert_eq!(Windows.file_name(r"C:\Users\alex\skills\review"), "review");
        assert_eq!(Windows.file_name("C:/Users/alex/skills/review"), "review");
        // A backslash is part of a name on a Posix Host.
        assert_eq!(Posix.file_name(r"/home/alex/a\b"), r"a\b");
    }

    #[test]
    fn a_path_walks_up_to_its_root() {
        assert_eq!(Posix.ancestors("/home/alex/app"), ["/home/alex/app", "/home/alex", "/home", "/"]);
        assert_eq!(Posix.ancestors("/home/alex/app/"), ["/home/alex/app", "/home/alex", "/home", "/"]);
        assert_eq!(Posix.ancestors("/"), ["/"]);
        assert_eq!(
            Windows.ancestors(r"C:\Users\alex\app"),
            [r"C:\Users\alex\app", r"C:\Users\alex", r"C:\Users", r"C:\"]
        );
        assert_eq!(Windows.ancestors(r"C:\"), [r"C:\"]);
        assert_eq!(
            Windows.ancestors(r"\\server\share\team\app"),
            [r"\\server\share\team\app", r"\\server\share\team", r"\\server\share\"]
        );
    }
}
