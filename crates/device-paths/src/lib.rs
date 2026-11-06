//! How a device writes its paths.
//!
//! The backend builds paths on a device, such as where an attachment is saved,
//! with the separator of the system the device's runner reports. A Windows device
//! shows `C:\Users\alex\.demi\attachments\…`, never `C:\Users\alex/.demi/…`.
//! Every path the backend or the runner joins, splits, walks up or writes from
//! `~` on a device goes through here.
//!
//! The crate reads the text of a path, which may name another machine's file,
//! and does no IO: it depends on no runtime and no other Demi crate, so both
//! the agent's side (`device-interface`) and the runner's wire
//! (`runner-protocol`) use it.

/// The path form of a device's system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePaths {
    /// Linux and macOS: `/` separates, and `/` is the root.
    #[default]
    Posix,
    /// Windows: `\` separates, though `/` is read as one too, and a path
    /// starts at a drive, `C:\`, or a share, `\\server\share\`.
    Windows,
}

impl DevicePaths {
    /// The separator the device writes.
    pub fn separator(self) -> char {
        match self {
            Self::Posix => '/',
            Self::Windows => '\\',
        }
    }

    /// Whether `c` separates components on the device.
    fn separates(self, c: char) -> bool {
        c == '/' || (self == Self::Windows && c == '\\')
    }

    /// The path of `name`, one component or a relative path written with
    /// `/`, in `directory`, with the device's separator throughout what is
    /// added.
    pub fn join(self, directory: &str, name: &str) -> String {
        let directory = directory.trim_end_matches(|c| self.separates(c));
        let name = match self {
            Self::Posix => name.to_owned(),
            Self::Windows => name.replace('/', "\\"),
        };
        format!("{directory}{}{name}", self.separator())
    }

    /// What `path` names under the device's home when it starts from `~`, as a
    /// shell reads it: `""` for `~` itself, `x` for `~/x`; none for any other
    /// path, `~x` among them.
    pub fn home_relative(self, path: &str) -> Option<&str> {
        match path.strip_prefix('~')? {
            "" => Some(""),
            rest if rest.starts_with(|c| self.separates(c)) => {
                Some(rest.trim_start_matches(|c| self.separates(c)))
            }
            _ => None,
        }
    }

    /// `path` with a leading `~` read as the device's home `home`
    /// ([`Self::home_relative`]); any other path as it is.
    pub fn from_home(self, home: &str, path: &str) -> String {
        match self.home_relative(path) {
            Some("") => home.to_owned(),
            Some(rest) => self.join(home, rest),
            None => path.to_owned(),
        }
    }

    /// `path` written from `~` when it is the device's home `home` or lies
    /// under it, which [`Self::from_home`] reads back: `~` for the home,
    /// `~` and the rest of the path, its separator first, for what is in it;
    /// any other path as it is. A Windows path's home is found as
    /// [`is_inside`] finds a directory, in either case and either separator.
    pub fn written_from_home(self, home: &str, path: &str) -> String {
        let home = home.trim_end_matches(|c| self.separates(c));
        let starts = match self {
            Self::Posix => path.starts_with(home),
            Self::Windows => path
                .get(..home.len())
                .is_some_and(|head| head.replace('/', "\\").eq_ignore_ascii_case(&home.replace('/', "\\"))),
        };
        if home.is_empty() || !starts {
            return path.to_owned();
        }
        let rest = path[home.len()..].trim_end_matches(|c| self.separates(c));
        if rest.is_empty() {
            "~".to_owned()
        } else if rest.starts_with(|c| self.separates(c)) {
            format!("~{rest}")
        } else {
            path.to_owned()
        }
    }

    /// Whether `path` is absolute on the device: from `/` on a Posix device,
    /// from a drive, `C:\`, or a share on Windows.
    pub fn is_absolute(self, path: &str) -> bool {
        let style = match self {
            Self::Posix => typed_path::PathType::Unix,
            Self::Windows => typed_path::PathType::Windows,
        };
        typed_path::Utf8TypedPath::new(path, style).is_absolute()
    }

    /// `path` read against `directory`, as a shell in `directory` reads it:
    /// as it is when it is absolute, otherwise under `directory`, with `.`
    /// and `..` taken out, in the device's spelling of paths.
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

    /// `path` read against `directory` as the device's own path join reads it,
    /// without taking out `.` and `..`, so the device, which follows links first,
    /// decides where they lead: an absolute path stays as it is, any other is
    /// joined to `directory`.
    pub fn beside(self, directory: &str, path: &str) -> String {
        if self.is_absolute(path) {
            path.to_owned()
        } else {
            self.join(directory, path)
        }
    }

    /// The names of the absolute `path` from its root, after `.` and `..`
    /// are taken out, joined by `/` as a URL's path names them: `/Users/alex`
    /// is `Users/alex`, `C:\Users\alex` is `C:/Users/alex` and
    /// `\\server\share\docs` is `//server/share/docs`. [`Self::at_place`]
    /// reads it back.
    pub fn place(self, path: &str) -> String {
        let path = self.resolve(self.root(path).as_str(), path);
        match self {
            Self::Posix => path.trim_start_matches('/').to_owned(),
            Self::Windows => path.replace('\\', "/"),
        }
    }

    /// The path a [`Self::place`] names; none for a place that starts at no
    /// drive or share on Windows, as a relative URL that climbs above the
    /// drive's root asks for.
    pub fn at_place(self, place: &str) -> Option<String> {
        let path = match self {
            Self::Posix => format!("/{place}"),
            Self::Windows => place.replace('/', "\\"),
        };
        self.is_absolute(&path).then_some(path)
    }

    /// The last component of `path`: the name of what it names.
    pub fn file_name(self, path: &str) -> &str {
        let path = path.trim_end_matches(|c| self.separates(c));
        path.rsplit(|c| self.separates(c)).next().unwrap_or(path)
    }

    /// `path` without the separators after its last component, so
    /// `/srv/app/` is `/srv/app`; a root keeps its own: `/`, `C:\`.
    pub fn trimmed(self, path: &str) -> &str {
        let trimmed = path.trim_end_matches(|c| self.separates(c));
        let root = self.root(path).len();
        if trimmed.len() < root { &path[..root] } else { trimmed }
    }

    /// The directory `path` is in; none for a root.
    pub fn parent(self, path: &str) -> Option<String> {
        self.ancestors(path).into_iter().nth(1)
    }

    /// `path` and every directory above it, nearest first, up to its root:
    /// `/` on a Posix device, the drive, `C:\`, or the share on Windows. A
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

/// Whether `path` is `directory` or inside it, judged by their parts after `.`
/// and `..` are taken out, without following a link
/// (`crates/agent-store/src/changes.rs`).
///
/// Both are read in the spelling `directory` has:
///
/// - A Windows one, by its drive or share, takes `\` and `/` as separators and
///   compares its parts as Windows does, ignoring case.
/// - A Posix one compares them exactly, a `\` part of a name.
///
/// A relative `path` is read under `directory`.
pub fn is_inside(directory: &str, path: &str) -> bool {
    use typed_path::{Utf8Component, Utf8TypedPath};
    match Utf8TypedPath::derive(directory) {
        Utf8TypedPath::Unix(directory) => {
            let path = directory.join(path).normalize();
            path.starts_with(directory.normalize())
        }
        Utf8TypedPath::Windows(directory) => {
            // A part as Windows compares it: a share's prefix with either
            // separator, and every part in either case.
            let parts = |path: &typed_path::Utf8WindowsPath| -> Vec<String> {
                path.components()
                    .map(|part| part.as_str().replace('/', "\\").to_lowercase())
                    .collect()
            };
            let path = directory.join(path).normalize();
            let directory = directory.normalize();
            let (path, directory) = (parts(&path), parts(&directory));
            path.starts_with(&directory)
        }
    }
}
