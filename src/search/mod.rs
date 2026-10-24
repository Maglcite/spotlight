//! Search snapshots: installed apps, a bounded file index, and web fallbacks,
//! ranked by one deterministic fuzzy scorer.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;
use url::Url;

mod apps;
pub use apps::discover_apps;

const MAX_RESULTS: usize = 12;
const MAX_INDEX_FILES: usize = 250_000;

/// A file from the index together with its precomputed, lowercased search
/// keys. The original filename and display path are re-derived from `path`
/// only for files that rank, so a large index does not carry display strings
/// that almost no query ever shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedFile {
    pub path: PathBuf,
    name_lower: String,
    display_lower: String,
}

impl IndexedFile {
    /// Index one path, shortening the home directory in the display key.
    pub fn new(path: PathBuf) -> Self {
        Self::with_home(path, dirs::home_dir().as_deref())
    }

    pub fn with_home(path: PathBuf, home: Option<&Path>) -> Self {
        let name_lower = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        let display_lower = display_path_from(&path, home).to_lowercase();
        Self {
            path,
            name_lower,
            display_lower,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub search_terms: String,
    pub icon: Option<String>,
    pub kind: ResultKind,
}

#[derive(Clone, Debug)]
pub enum ResultKind {
    /// `desktop` is the `.desktop` file on Linux and the Start Menu shortcut
    /// (`.lnk`, `.url`, `.appref-ms`) on Windows.
    App {
        desktop: PathBuf,
    },
    File {
        path: PathBuf,
    },
    Web {
        url: String,
    },
}

/// Build a bounded, best-effort file index. `ignore` handles standard ignore
/// files while these defaults keep hidden and generated trees out of the index.
pub fn build_file_index(roots: &[PathBuf]) -> Vec<IndexedFile> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let home = dirs::home_dir();
    for root in roots {
        if files.len() >= MAX_INDEX_FILES {
            break;
        }
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(true)
            .follow_links(false)
            .same_file_system(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .filter_entry(|entry| {
                !excluded_path(
                    entry.path(),
                    entry.file_type().is_some_and(|kind| kind.is_dir()),
                )
            });
        for result in builder.build() {
            if files.len() >= MAX_INDEX_FILES {
                break;
            }
            let Ok(entry) = result else { continue };
            if !entry.file_type().is_some_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.into_path();
            if seen.insert(path.clone()) {
                files.push(IndexedFile::with_home(path, home.as_deref()));
            }
        }
    }
    files
}

/// Generated, cache and tool directories that are never worth indexing.
const EXCLUDED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "Caches",
    "cache",
    "vendor",
    ".git",
];

/// Windows profile trees that hold application state rather than documents,
/// plus the legacy compatibility junctions that deny listing.
#[cfg(windows)]
const EXCLUDED_WINDOWS_DIRS: &[&str] = &[
    "AppData",
    "$Recycle.Bin",
    "System Volume Information",
    "Application Data",
    "Local Settings",
    "My Documents",
    "NetHood",
    "PrintHood",
    "Recent",
    "SendTo",
    "Start Menu",
    "Templates",
    "Cookies",
];

fn excluded_path(path: &Path, is_dir: bool) -> bool {
    if !is_dir {
        return false;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    #[cfg(windows)]
    {
        // NTFS names are case-insensitive.
        EXCLUDED_DIRS
            .iter()
            .chain(EXCLUDED_WINDOWS_DIRS)
            .any(|excluded| excluded.eq_ignore_ascii_case(name))
    }
    #[cfg(not(windows))]
    {
        EXCLUDED_DIRS.contains(&name)
    }
}

/// Human-friendly path for subtitles: the home directory becomes `~`
/// (`~/Documents/x` on Linux, `~\Documents\x` on Windows).
pub fn display_path(path: &Path) -> String {
    display_path_from(path, dirs::home_dir().as_deref())
}

fn display_path_from(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

/// Search the prepared application and file snapshots. The caller can run this
/// function on a worker thread; it performs no UI or blocking process work.
/// The query is lowercased once; file candidates are already lowercased keys.
pub fn query(
    query: &str,
    apps: &[SearchResult],
    files: &[IndexedFile],
    web_engine: &str,
) -> Vec<SearchResult> {
    let needle = query.trim();
    if needle.is_empty() {
        return apps.iter().take(MAX_RESULTS).cloned().collect();
    }
    if let Some(url) = explicit_http_url(needle) {
        return vec![SearchResult {
            id: format!("web:{url}"),
            title: url.clone(),
            subtitle: "Open website".into(),
            search_terms: String::new(),
            icon: None,
            kind: ResultKind::Web { url },
        }];
    }

    let needle_lower = needle.to_lowercase();
    let home = dirs::home_dir();
    let mut ranked: Vec<(u8, i32, SearchResult)> = Vec::new();
    for app in apps {
        let score = best_score(
            &needle_lower,
            &[&app.title, &app.id, &app.subtitle, &app.search_terms],
        );
        if score > 0 {
            offer(&mut ranked, (0, score, app.clone()));
        }
    }
    for file in files {
        // Match the shortened path so the home directory itself (often the
        // user's name) does not make every file match.
        let score = score_keys(&needle_lower, &file.name_lower)
            .max(score_keys(&needle_lower, &file.display_lower));
        if score > 0 {
            offer(&mut ranked, (1, score, file_result(file, home.as_deref())));
        }
    }

    let mut results = ranked
        .into_iter()
        .take(MAX_RESULTS - 1)
        .map(|(_, _, result)| result)
        .collect::<Vec<_>>();
    if let Some(url) = web_search_url(needle, web_engine) {
        results.push(SearchResult {
            id: format!("web-search:{url}"),
            title: format!("Search the web for ‘{needle}’"),
            search_terms: String::new(),
            subtitle: match web_engine.to_ascii_lowercase().as_str() {
                "google" => "Google".into(),
                "brave" => "Brave Search".into(),
                _ => "DuckDuckGo".into(),
            },
            icon: None,
            kind: ResultKind::Web { url },
        });
    }
    results
}

/// Original-cased strings are built only for files that made the ranked list.
fn file_result(file: &IndexedFile, home: Option<&Path>) -> SearchResult {
    let path = &file.path;
    SearchResult {
        id: format!("file:{}", path.display()),
        title: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        subtitle: display_path_from(path, home),
        search_terms: String::new(),
        icon: None,
        kind: ResultKind::File { path: path.clone() },
    }
}

fn offer(ranked: &mut Vec<(u8, i32, SearchResult)>, candidate: (u8, i32, SearchResult)) {
    let index = ranked.partition_point(|existing| {
        candidate
            .1
            .cmp(&existing.1)
            .then_with(|| existing.0.cmp(&candidate.0))
            .then_with(|| {
                existing
                    .2
                    .title
                    .to_lowercase()
                    .cmp(&candidate.2.title.to_lowercase())
            })
            != std::cmp::Ordering::Greater
    });
    if index >= MAX_RESULTS - 1 {
        return;
    }
    ranked.insert(index, candidate);
    ranked.truncate(MAX_RESULTS - 1);
}

fn explicit_http_url(value: &str) -> Option<String> {
    let lower = value
        .get(..value.find(':')?)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if lower != "http" && lower != "https" {
        return None;
    }
    let parsed = Url::parse(value).ok()?;
    parsed.host_str()?;
    Some(parsed.to_string())
}

fn web_search_url(query: &str, engine: &str) -> Option<String> {
    let (base, key) = match engine.to_ascii_lowercase().as_str() {
        "google" => ("https://www.google.com/search", "q"),
        "brave" => ("https://search.brave.com/search", "q"),
        _ => ("https://duckduckgo.com/", "q"),
    };
    let mut url = Url::parse(base).ok()?;
    url.query_pairs_mut().append_pair(key, query);
    Some(url.to_string())
}

fn best_score(query_lower: &str, candidates: &[&str]) -> i32 {
    candidates
        .iter()
        .map(|candidate| score_keys(query_lower, &candidate.to_lowercase()))
        .max()
        .unwrap_or(0)
}

/// Deterministic case-insensitive fuzzy score over text that is already
/// lowercased: exact, prefix, word prefix, contiguous substring, then ordered
/// subsequence. File candidates arrive as precomputed keys; app candidates
/// are lowercased per query by [`best_score`].
fn score_keys(query_lower: &str, candidate_lower: &str) -> i32 {
    let q = query_lower;
    let c = candidate_lower;
    if q.is_empty() {
        return 0;
    }
    if c == q {
        return 10_000;
    }
    if c.starts_with(q) {
        return 8_000 - c.len().min(1000) as i32;
    }
    if c.split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word.starts_with(q))
    {
        return 6_000 - c.len().min(1000) as i32;
    }
    if let Some(index) = c.find(q) {
        return 4_000 - index.min(1000) as i32;
    }
    let mut chars = c.chars().enumerate();
    let mut positions = Vec::new();
    for wanted in q.chars() {
        let Some((index, _)) = chars.find(|(_, actual)| *actual == wanted) else {
            return 0;
        };
        positions.push(index);
    }
    let span = positions.last().copied().unwrap_or(0) - positions.first().copied().unwrap_or(0) + 1;
    2_000 - span.min(1_999) as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fuzzy_order_and_result_limit() {
        let app = SearchResult {
            id: "org.example.Editor".into(),
            title: "Editor".into(),
            subtitle: "Text editor".into(),
            search_terms: String::new(),
            icon: None,
            kind: ResultKind::App {
                desktop: "editor.desktop".into(),
            },
        };
        let files = [
            IndexedFile::with_home(PathBuf::from("/tmp/edit.txt"), None),
            IndexedFile::with_home(PathBuf::from("/tmp/other"), None),
        ];
        let results = query("edit", &[app], &files, "duckduckgo");
        assert!(matches!(results[0].kind, ResultKind::App { .. }));
        assert!(matches!(results[1].kind, ResultKind::File { .. }));
        assert!(matches!(
            results.last().unwrap().kind,
            ResultKind::Web { .. }
        ));
    }

    #[test]
    fn mixed_case_queries_keep_the_exact_ranking_and_original_strings() {
        let files = [
            IndexedFile::with_home(PathBuf::from("/tmp/Notes-Archive.TXT"), None),
            IndexedFile::with_home(PathBuf::from("/tmp/notes.md"), None),
            IndexedFile::with_home(PathBuf::from("/tmp/OTHER.txt"), None),
        ];
        assert_eq!(files[0].name_lower, "notes-archive.txt");
        assert_eq!(files[0].display_lower, "/tmp/notes-archive.txt");

        let results = query("NOTES", &[], &files, "duckduckgo");
        let titles: Vec<&str> = results.iter().map(|r| r.title.as_str()).collect();
        // The shorter prefix hit outranks the longer one; the unrelated file
        // never makes the list, so it never allocates result strings.
        assert_eq!(&titles[..2], ["notes.md", "Notes-Archive.TXT"]);
        assert!(!titles.iter().any(|title| title.contains("OTHER")));
        // Result strings keep the path's original casing.
        let archive = results
            .iter()
            .find(|r| r.title == "Notes-Archive.TXT")
            .unwrap();
        assert_eq!(archive.subtitle, "/tmp/Notes-Archive.TXT");
        // The web fallback keeps the query text as typed.
        let web = results.last().unwrap();
        assert!(web.title.contains("NOTES"));
    }

    #[test]
    fn word_prefix_and_subsequence_classes_still_rank_in_order() {
        let files = [
            IndexedFile::with_home(PathBuf::from("/tmp/floor-plan.pdf"), None),
            IndexedFile::with_home(PathBuf::from("/tmp/plan.txt"), None),
            IndexedFile::with_home(PathBuf::from("/tmp/poland.pdf"), None),
        ];
        let results = query("PLAN", &[], &files, "duckduckgo");
        let titles: Vec<&str> = results.iter().map(|r| r.title.as_str()).collect();
        // Prefix (plan.txt) beats word prefix (floor-plan.pdf) beats the
        // ordered-subsequence match (poland.pdf), all case-insensitively.
        assert_eq!(&titles[..3], ["plan.txt", "floor-plan.pdf", "poland.pdf"]);
    }

    #[test]
    fn url_query_is_encoded_and_http_is_an_action() {
        let result = query("two words & tea", &[], &[], "google").pop().unwrap();
        let ResultKind::Web { url } = result.kind else {
            panic!()
        };
        assert!(url.contains("two+words+%26+tea"));
        let explicit = query("https://example.com/path?q=a%20b", &[], &[], "duckduckgo");
        assert_eq!(explicit.len(), 1);
        assert!(matches!(explicit[0].kind, ResultKind::Web { .. }));
    }

    #[test]
    fn index_skips_hidden_generated_and_symlink_directories() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("src")).unwrap();
        fs::create_dir_all(root.path().join("target")).unwrap();
        fs::create_dir_all(root.path().join(".hidden")).unwrap();
        fs::write(root.path().join("src/main.rs"), "").unwrap();
        fs::write(root.path().join("target/out"), "").unwrap();
        fs::write(root.path().join(".hidden/secret"), "").unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("linked"), "").unwrap();
        // Symlinks need privileges on Windows; the unit test covers them on Unix.
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), root.path().join("linked-dir")).unwrap();
        let index = build_file_index(&[root.path().to_path_buf()]);
        assert_eq!(
            index.iter().map(|file| &file.path).collect::<Vec<_>>(),
            vec![&root.path().join("src/main.rs")]
        );
        // Keys are precomputed and lowercased at index time.
        assert_eq!(index[0].name_lower, "main.rs");
    }

    #[test]
    fn home_paths_are_shortened_for_display() {
        let home = std::env::temp_dir().join("spotlight-home");
        let file = home.join("Documents").join("notes.txt");
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            display_path_from(&file, Some(&home)),
            format!("~{sep}Documents{sep}notes.txt")
        );
        assert_eq!(display_path_from(&home, Some(&home)), "~");
        assert_eq!(display_path_from(&file, None), file.display().to_string());
    }
}
