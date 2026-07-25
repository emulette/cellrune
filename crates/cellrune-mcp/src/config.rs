use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, File};

use crate::error::McpError;

/// Default maximum number of resident workbook sessions.
pub const DEFAULT_MAX_SESSIONS: usize = 16;
/// Hard upper bound for configured resident workbook sessions.
pub const MAX_SESSIONS: usize = 256;
/// Default idle session lifetime in seconds.
pub const DEFAULT_SESSION_TTL_SECONDS: u64 = 1_800;
/// Hard upper bound for an idle session lifetime.
pub const MAX_SESSION_TTL_SECONDS: u64 = 86_400;
/// Default maximum serialized structured payload size.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 1_048_576;
/// Hard upper bound for a serialized structured payload.
pub const MAX_RESPONSE_BYTES: usize = 16_777_216;
/// Default maximum accepted workbook file size.
pub const DEFAULT_MAX_WORKBOOK_BYTES: u64 = 104_857_600;
/// Hard upper bound for an accepted workbook file.
pub const MAX_WORKBOOK_BYTES: u64 = 1_073_741_824;

const MIN_RESPONSE_BYTES: usize = 1_024;
const DETAIL_MAX_SESSIONS: &str = "max_sessions must be between 1 and 256";
const DETAIL_SESSION_TTL: &str = "session_ttl_seconds must be between 1 and 86400";
const DETAIL_RESPONSE_BYTES: &str = "max_response_bytes must be between 1024 and 16777216";
const DETAIL_WORKBOOK_BYTES: &str = "max_workbook_bytes must be between 1 and 1073741824";
const DETAIL_PATH_METADATA: &str = "workbook path metadata is unavailable";
const DETAIL_DESTINATION_PARENT: &str =
    "workbook destination must have an existing parent directory";

#[derive(Debug, Clone)]
struct AllowedRoot {
    canonical_path: PathBuf,
    match_paths: Vec<PathBuf>,
    directory: Arc<Dir>,
}

/// Open workbook input bound to the approved directory and file identities validated by policy.
#[derive(Debug)]
pub struct ResolvedInput {
    display_path: PathBuf,
    file: File,
    maximum_bytes: u64,
}

impl ResolvedInput {
    /// Returns the canonical ambient path used for diagnostics.
    pub fn display_path(&self) -> &Path {
        &self.display_path
    }

    /// Returns the byte limit shared by bounded file reading and workbook package parsing.
    pub const fn maximum_bytes(&self) -> u64 {
        self.maximum_bytes
    }

    /// Reads the already-open file handle without accepting more than the configured byte limit.
    ///
    /// The input is consumed so every byte comes from the same capability-bound handle validated
    /// by [`ServerConfig::resolve_input`]. Reading stops after at most the configured maximum plus
    /// one byte, allowing growth after the initial metadata check to be rejected without an
    /// unbounded allocation.
    ///
    /// # Errors
    ///
    /// Returns a stable path error when the open handle cannot be read or a size-policy error when
    /// the handle contains more than the configured maximum.
    pub fn read_bytes(self) -> Result<Vec<u8>, McpError> {
        let read_limit = self.maximum_bytes + 1;
        let mut bytes = Vec::new();
        self.file
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| McpError::path_invalid(path_detail(&self.display_path, &error)))?;
        let actual_bytes = bytes.len() as u64;
        if actual_bytes > self.maximum_bytes {
            return Err(McpError::input_too_large(actual_bytes, self.maximum_bytes));
        }
        Ok(bytes)
    }
}

/// Save As destination bound to the directory identity validated by server policy.
#[derive(Debug)]
pub struct ResolvedOutput {
    display_path: PathBuf,
    parent: Dir,
    file_name: PathBuf,
}

impl ResolvedOutput {
    /// Returns the normalized ambient path used in the MCP response.
    pub fn display_path(&self) -> &Path {
        &self.display_path
    }

    /// Returns the open destination-parent capability retained across Save As preparation.
    pub fn parent(&self) -> &Dir {
        &self.parent
    }

    /// Returns the single relative file name to install beneath [`Self::parent`].
    pub fn file_name(&self) -> &Path {
        &self.file_name
    }
}

/// Validated local-only MCP server policy.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    roots: Vec<PathBuf>,
    allowed_roots: Vec<AllowedRoot>,
    max_sessions: usize,
    session_ttl: Duration,
    max_response_bytes: usize,
    max_workbook_bytes: u64,
    allow_overwrite: bool,
}

impl ServerConfig {
    /// Validates limits and canonicalizes the allowed workbook roots.
    ///
    /// # Errors
    ///
    /// Returns a stable configuration or path error when a root or limit is invalid.
    pub fn new(
        roots: Vec<PathBuf>,
        max_sessions: usize,
        session_ttl_seconds: u64,
        max_response_bytes: usize,
        max_workbook_bytes: u64,
        allow_overwrite: bool,
    ) -> Result<Self, McpError> {
        if roots.is_empty() {
            return Err(McpError::root_required());
        }
        if !(1..=MAX_SESSIONS).contains(&max_sessions) {
            return Err(McpError::config_limit(DETAIL_MAX_SESSIONS.to_owned()));
        }
        if !(1..=MAX_SESSION_TTL_SECONDS).contains(&session_ttl_seconds) {
            return Err(McpError::config_limit(DETAIL_SESSION_TTL.to_owned()));
        }
        if !(MIN_RESPONSE_BYTES..=MAX_RESPONSE_BYTES).contains(&max_response_bytes) {
            return Err(McpError::config_limit(DETAIL_RESPONSE_BYTES.to_owned()));
        }
        if !(1..=MAX_WORKBOOK_BYTES).contains(&max_workbook_bytes) {
            return Err(McpError::config_limit(DETAIL_WORKBOOK_BYTES.to_owned()));
        }

        let mut root_aliases = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
        for root in roots {
            let absolute = std::path::absolute(&root)
                .map_err(|error| McpError::root_invalid(path_detail(&root, &error)))?;
            let canonical = fs::canonicalize(&root)
                .map_err(|error| McpError::root_invalid(path_detail(&root, &error)))?;
            let metadata = fs::metadata(&canonical)
                .map_err(|error| McpError::root_invalid(path_detail(&canonical, &error)))?;
            if !metadata.is_dir() {
                return Err(McpError::root_invalid(canonical.display().to_string()));
            }
            let aliases = root_aliases.entry(canonical.clone()).or_default();
            aliases.insert(absolute);
            aliases.insert(canonical);
        }

        let roots = root_aliases.keys().cloned().collect::<Vec<_>>();
        let allowed_roots = root_aliases
            .into_iter()
            .map(|(path, match_paths)| -> Result<AllowedRoot, McpError> {
                let directory = Dir::open_ambient_dir(&path, ambient_authority())
                    .map_err(|error| McpError::root_invalid(path_detail(&path, &error)))?;
                Ok(AllowedRoot {
                    canonical_path: path,
                    match_paths: match_paths.into_iter().collect(),
                    directory: Arc::new(directory),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            roots,
            allowed_roots,
            max_sessions,
            session_ttl: Duration::from_secs(session_ttl_seconds),
            max_response_bytes,
            max_workbook_bytes,
            allow_overwrite,
        })
    }

    /// Returns the canonical allowed roots.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Returns the resident workbook-session limit.
    pub const fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    /// Returns the idle workbook-session lifetime.
    pub const fn session_ttl(&self) -> Duration {
        self.session_ttl
    }

    /// Returns the maximum serialized structured payload size.
    pub const fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    #[cfg(test)]
    pub(crate) const fn with_test_response_bytes(mut self, maximum: usize) -> Self {
        self.max_response_bytes = maximum;
        self
    }

    /// Returns whether explicit overwrite requests may replace existing files.
    pub const fn allow_overwrite(&self) -> bool {
        self.allow_overwrite
    }

    /// Resolves and validates an existing workbook input file.
    ///
    /// # Errors
    ///
    /// Returns a stable path or size-policy error.
    pub fn resolve_input(&self, requested: &str) -> Result<ResolvedInput, McpError> {
        let requested = absolute_path(requested)?;
        let canonical = fs::canonicalize(&requested)
            .map_err(|error| McpError::path_invalid(path_detail(&requested, &error)))?;
        let root = self
            .allowed_roots
            .iter()
            .filter(|root| canonical.starts_with(&root.canonical_path))
            .max_by_key(|root| root.canonical_path.components().count())
            .ok_or_else(McpError::path_outside_root)?;
        let relative = canonical
            .strip_prefix(&root.canonical_path)
            .map_err(|error| McpError::path_invalid(error.to_string()))?;
        if relative.as_os_str().is_empty() {
            return Err(McpError::input_not_file());
        }
        let file = root
            .directory
            .open(relative)
            .map_err(|error| McpError::path_invalid(path_detail(&canonical, &error)))?;
        let metadata = file
            .metadata()
            .map_err(|error| McpError::path_invalid(path_detail(&canonical, &error)))?;
        if !metadata.is_file() {
            return Err(McpError::input_not_file());
        }
        if metadata.len() > self.max_workbook_bytes {
            return Err(McpError::input_too_large(
                metadata.len(),
                self.max_workbook_bytes,
            ));
        }
        Ok(ResolvedInput {
            display_path: canonical,
            file,
            maximum_bytes: self.max_workbook_bytes,
        })
    }

    /// Resolves a safe Save As destination under an allowed root.
    ///
    /// # Errors
    ///
    /// Returns a stable path or overwrite-policy error.
    pub fn resolve_output(
        &self,
        requested: &str,
        replace_existing: bool,
    ) -> Result<ResolvedOutput, McpError> {
        if replace_existing && !self.allow_overwrite {
            return Err(McpError::overwrite_disallowed());
        }
        let requested = absolute_path(requested)?;
        let (root, relative) = self
            .allowed_roots
            .iter()
            .flat_map(|root| {
                root.match_paths
                    .iter()
                    .map(move |match_path| (root, match_path))
            })
            .filter_map(|(root, match_path)| {
                relative_within(&requested, match_path)
                    .map(|relative| (root, match_path.components().count(), relative))
            })
            .max_by_key(|(_, depth, _)| *depth)
            .map(|(root, _, relative)| (root, relative))
            .ok_or_else(McpError::path_outside_root)?;
        let relative = relative.as_path();
        if relative.as_os_str().is_empty()
            || !relative
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(McpError::path_invalid(DETAIL_DESTINATION_PARENT.to_owned()));
        }
        let file_name = relative
            .file_name()
            .map(PathBuf::from)
            .ok_or_else(|| McpError::path_invalid(DETAIL_DESTINATION_PARENT.to_owned()))?;
        let relative_parent = relative.parent().unwrap_or_else(|| Path::new(""));
        let parent = if relative_parent.as_os_str().is_empty() {
            root.directory
                .try_clone()
                .map_err(|error| McpError::path_invalid(path_detail(&requested, &error)))?
        } else {
            root.directory
                .open_dir(relative_parent)
                .map_err(|error| McpError::path_invalid(path_detail(&requested, &error)))?
        };
        match parent.symlink_metadata(&file_name) {
            Ok(metadata) if metadata.is_dir() => {
                return Err(McpError::path_invalid(DETAIL_PATH_METADATA.to_owned()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(McpError::path_invalid(path_detail(&requested, &error)));
            }
        }
        Ok(ResolvedOutput {
            // Reported against the canonical root, matching `resolve_input`, so a
            // destination is displayed the same way whichever alias named it.
            display_path: root.canonical_path.join(relative),
            parent,
            file_name,
        })
    }
}

/// Returns the part of `path` below `root`, or `None` when `path` is not inside it.
///
/// A Save As destination need not exist yet, so it cannot be canonicalized before
/// this comparison. On Windows that leaves the caller's casing intact, and an
/// exact match would reject a destination that names an allowed root with
/// different casing even though it resolves to the same directory. Components are
/// therefore compared the way the host filesystem compares them.
///
/// This only selects which root a request belongs to. Containment is still
/// enforced afterwards by reopening the destination through that root's directory
/// capability, which is what makes the boundary hold.
fn relative_within(path: &Path, root: &Path) -> Option<PathBuf> {
    let mut components = path.components();
    for expected in root.components() {
        let actual = components.next()?;
        if !components_match(&actual, &expected) {
            return None;
        }
    }
    Some(components.as_path().to_path_buf())
}

#[cfg(windows)]
fn components_match(actual: &Component<'_>, expected: &Component<'_>) -> bool {
    match (actual, expected) {
        (Component::Normal(left), Component::Normal(right)) => left.eq_ignore_ascii_case(right),
        (Component::Prefix(left), Component::Prefix(right)) => {
            left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
        }
        _ => actual == expected,
    }
}

#[cfg(not(windows))]
fn components_match(actual: &Component<'_>, expected: &Component<'_>) -> bool {
    actual == expected
}

fn absolute_path(requested: &str) -> Result<PathBuf, McpError> {
    let path = PathBuf::from(requested);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(McpError::path_absolute())
    }
}

fn path_detail(path: &Path, error: &io::Error) -> String {
    format!("{}: {error}", path.display())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock must follow the Unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cellrune-mcp-{label}-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("test directory must be created");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn config(root: &Path) -> ServerConfig {
        config_with_workbook_limit(root, DEFAULT_MAX_WORKBOOK_BYTES)
    }

    fn config_with_workbook_limit(root: &Path, maximum_bytes: u64) -> ServerConfig {
        ServerConfig::new(
            vec![root.to_owned()],
            DEFAULT_MAX_SESSIONS,
            DEFAULT_SESSION_TTL_SECONDS,
            DEFAULT_MAX_RESPONSE_BYTES,
            maximum_bytes,
            false,
        )
        .expect("test configuration must be valid")
    }

    #[test]
    fn input_paths_must_resolve_inside_a_root() {
        let allowed = TestDirectory::new("allowed");
        let outside = TestDirectory::new("outside");
        let workbook = outside.path.join("outside.xlsx");
        fs::write(&workbook, b"not a workbook").expect("fixture must be written");

        let error = config(&allowed.path)
            .resolve_input(workbook.to_str().expect("path must be UTF-8"))
            .expect_err("outside path must fail");

        assert_eq!(error.payload().code, "mcp.path.outside_root");
    }

    #[test]
    fn output_overwrite_requires_server_opt_in() {
        let allowed = TestDirectory::new("overwrite");
        let workbook = allowed.path.join("output.xlsx");
        fs::write(&workbook, b"existing").expect("fixture must be written");

        let error = config(&allowed.path)
            .resolve_output(workbook.to_str().expect("path must be UTF-8"), true)
            .expect_err("overwrite must be denied");

        assert_eq!(error.payload().code, "mcp.output.overwrite_disallowed");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_input_cannot_escape_an_allowed_root() {
        use std::os::unix::fs::symlink;

        let allowed = TestDirectory::new("symlink-allowed");
        let outside = TestDirectory::new("symlink-outside");
        let outside_workbook = outside.path.join("outside.xlsx");
        fs::write(&outside_workbook, b"outside root").expect("fixture must be written");
        let link = allowed.path.join("linked.xlsx");
        symlink(&outside_workbook, &link).expect("test symlink must be created");

        let error = config(&allowed.path)
            .resolve_input(link.to_str().expect("path must be UTF-8"))
            .expect_err("symlink escape must fail");

        assert_eq!(error.payload().code, "mcp.path.outside_root");
    }

    #[cfg(unix)]
    #[test]
    fn resolved_input_retains_the_validated_file_identity() {
        use std::os::unix::fs::symlink;

        let allowed = TestDirectory::new("input-file-identity");
        let outside = TestDirectory::new("input-file-identity-outside");
        let requested = allowed.path.join("input.xlsx");
        let parked = allowed.path.join("parked.xlsx");
        let outside_workbook = outside.path.join("outside.xlsx");
        fs::write(&requested, b"validated contents").expect("input fixture must be written");
        fs::write(&outside_workbook, b"outside replacement")
            .expect("outside fixture must be written");
        let input = config(&allowed.path)
            .resolve_input(requested.to_str().expect("path must be UTF-8"))
            .expect("input must resolve");

        fs::rename(&requested, &parked).expect("validated input must be parked");
        symlink(&outside_workbook, &requested).expect("ambient input path must be replaced");

        assert_eq!(
            input
                .read_bytes()
                .expect("open input handle must remain readable"),
            b"validated contents"
        );
    }

    #[cfg(unix)]
    #[test]
    fn input_resolution_prefers_the_longest_canonical_root_capability() {
        let outer = TestDirectory::new("input-nested-root");
        let nested = outer.path.join("nested");
        let parked = outer.path.join("parked");
        let requested = nested.join("input.xlsx");
        fs::create_dir(&nested).expect("nested root must be created");
        fs::write(&requested, b"nested root contents").expect("nested fixture must be written");
        let config = ServerConfig::new(
            vec![outer.path.clone(), nested.clone()],
            DEFAULT_MAX_SESSIONS,
            DEFAULT_SESSION_TTL_SECONDS,
            DEFAULT_MAX_RESPONSE_BYTES,
            DEFAULT_MAX_WORKBOOK_BYTES,
            false,
        )
        .expect("nested-root configuration must be valid");

        fs::rename(&nested, &parked).expect("nested root must be parked");
        fs::create_dir(&nested).expect("ambient nested path must be replaced");
        fs::write(&requested, b"outer root replacement")
            .expect("replacement fixture must be written");
        let input = config
            .resolve_input(requested.to_str().expect("path must be UTF-8"))
            .expect("input must resolve through the nested root");

        assert_eq!(
            input
                .read_bytes()
                .expect("nested root handle must remain readable"),
            b"nested root contents"
        );
    }

    #[test]
    fn resolved_input_rejects_growth_after_the_metadata_check() {
        let allowed = TestDirectory::new("input-growth-limit");
        let workbook = allowed.path.join("input.xlsx");
        fs::write(&workbook, b"1234").expect("input fixture must be written");
        let input = config_with_workbook_limit(&allowed.path, 4)
            .resolve_input(workbook.to_str().expect("path must be UTF-8"))
            .expect("input at the exact limit must resolve");
        assert_eq!(input.maximum_bytes(), 4);

        fs::OpenOptions::new()
            .append(true)
            .open(&workbook)
            .expect("input must reopen for append")
            .write_all(b"5")
            .expect("input must grow");
        let error = input
            .read_bytes()
            .expect_err("growth beyond the configured limit must fail");

        assert_eq!(error.payload().code, "mcp.input.byte_limit_exceeded");
        assert_eq!(error.payload().details.actual_bytes, Some(5));
        assert_eq!(error.payload().details.maximum_bytes, Some(4));
    }

    #[cfg(windows)]
    #[test]
    fn output_destination_accepts_a_case_variant_of_an_allowed_root() {
        let allowed = TestDirectory::new("output-case-variant");
        let requested = allowed
            .path
            .to_str()
            .expect("path must be UTF-8")
            .to_uppercase()
            + "\\output.xlsx";

        let output = config(&allowed.path)
            .resolve_output(&requested, false)
            .expect("a case variant of an allowed root resolves to the same directory");

        output
            .parent()
            .write(output.file_name(), b"bound")
            .expect("resolved parent capability must be usable");
        assert_eq!(
            fs::read(allowed.path.join("output.xlsx")).expect("output must exist"),
            b"bound"
        );
    }

    #[test]
    fn output_destination_outside_every_root_is_rejected() {
        let allowed = TestDirectory::new("output-outside-allowed");
        let outside = TestDirectory::new("output-outside-other");
        let requested = outside.path.join("output.xlsx");

        let error = config(&allowed.path)
            .resolve_output(requested.to_str().expect("path must be UTF-8"), false)
            .expect_err("a destination outside every root must fail");

        assert_eq!(error.payload().code, "mcp.path.outside_root");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_output_parent_cannot_escape_an_allowed_root() {
        use std::os::unix::fs::symlink;

        let allowed = TestDirectory::new("output-symlink-allowed");
        let outside = TestDirectory::new("output-symlink-outside");
        let link = allowed.path.join("outside");
        symlink(&outside.path, &link).expect("test symlink must be created");
        let workbook = link.join("output.xlsx");

        let error = config(&allowed.path)
            .resolve_output(workbook.to_str().expect("path must be UTF-8"), false)
            .expect_err("symlinked parent escape must fail");

        assert_eq!(error.payload().code, "mcp.path.invalid");
    }

    #[cfg(unix)]
    #[test]
    fn resolved_output_retains_the_validated_parent_identity() {
        use std::os::unix::fs::symlink;

        let allowed = TestDirectory::new("output-parent-identity");
        let live_parent = allowed.path.join("live");
        let parked_parent = allowed.path.join("parked");
        let outside_parent = allowed.path.join("outside");
        fs::create_dir(&live_parent).expect("live parent must be created");
        fs::create_dir(&outside_parent).expect("outside parent must be created");
        let requested = live_parent.join("output.xlsx");
        let output = config(&allowed.path)
            .resolve_output(requested.to_str().expect("path must be UTF-8"), false)
            .expect("output must resolve");

        fs::rename(&live_parent, &parked_parent).expect("validated parent must be parked");
        symlink(&outside_parent, &live_parent).expect("ambient path must be replaced");
        output
            .parent()
            .write(output.file_name(), b"bound")
            .expect("open parent capability must remain usable");

        assert_eq!(
            fs::read(parked_parent.join("output.xlsx")).expect("bound output must exist"),
            b"bound"
        );
        assert!(
            !outside_parent.join("output.xlsx").exists(),
            "ambient path replacement must not redirect the output"
        );
    }
}
