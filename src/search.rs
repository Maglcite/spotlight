use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use freedesktop_desktop_entry::{
    DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
};
use ignore::WalkBuilder;
use url::Url;

const MAX_RESULTS: usize = 12;
const MAX_INDEX_FILES: usize = 250_000;

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
    App { desktop: PathBuf },
    File { path: PathBuf },
    Web { url: String },
}

/// Discover installed desktop applications. Search roots are kept in XDG priority
/// order so a user's entry shadows a system entry with the same desktop ID.
pub fn discover_apps() -> Vec<SearchResult> {
    let locales = get_languages_from_env();
    let roots = app_roots();
    let desktops = current_desktop().unwrap_or_default();
    let mut winners: HashMap<String, DesktopEntry> = HashMap::new();

    for entry in Iter::new(roots.into_iter()).entries(Some(&locales)) {
        // The first path encountered wins. Do this before visibility filtering:
        // Hidden=true user entries must still hide the lower-priority system app.
        if winners.contains_key(entry.id()) {
            continue;
        }
        winners.insert(entry.id().to_owned(), entry);
    }

    let mut apps = winners
        .into_values()
        .filter(|entry| visible_on_this_desktop(entry, &desktops))
        .filter_map(|entry| {
            let title = entry.name(&locales)?.into_owned();
            if title.trim().is_empty() || entry.type_().is_some_and(|kind| kind != "Application") {
                return None;
            }
            if let Some(try_exec) = entry.try_exec()
                && !executable_exists(try_exec)
            {
                return None;
            }
            let desktop = entry.path.clone();
            let id = entry.id().to_owned();
            let subtitle = entry
                .comment(&locales)
                .map(|s| s.into_owned())
                .unwrap_or_else(|| id.clone());
            let search_terms = format!(
                "{} {} {}",
                entry.generic_name(&locales).unwrap_or_default(),
                entry.keywords(&locales).unwrap_or_default().join(" "),
                entry.exec().unwrap_or_default()
            );
            Some(SearchResult {
                search_terms,
                id,
                title,
                subtitle,
                icon: entry.icon().map(str::to_owned),
                kind: ResultKind::App { desktop },
            })
        })
        .collect::<Vec<_>>();
    apps.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    apps
}

fn app_roots() -> Vec<PathBuf> {
    let mut roots = default_paths().collect::<Vec<_>>();
    if let Some(home) = env::var_os("HOME") {
        roots.push(PathBuf::from(home).join(".local/share/flatpak/exports/share/applications"));
    }
    roots.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(path.clone()));
    roots
}

fn visible_on_this_desktop(entry: &DesktopEntry, desktops: &[String]) -> bool {
    if entry.hidden() || entry.no_display() {
        return false;
    }
    let current = desktops
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect::<HashSet<_>>();
    if entry.only_show_in().is_some_and(|allowed| {
        !allowed
            .iter()
            .any(|name| current.contains(&name.to_ascii_lowercase()))
    }) {
        return false;
    }
    if entry.not_show_in().is_some_and(|blocked| {
        blocked
            .iter()
            .any(|name| current.contains(&name.to_ascii_lowercase()))
    }) {
        return false;
    }
    true
}

fn executable_exists(program: &str) -> bool {
    let path = Path::new(program);
    if path.components().count() > 1 {
        return path.is_file();
    }
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// Build a bounded, best-effort file index. `ignore` handles standard ignore
/// files while these defaults keep hidden and generated trees out of the index.
pub fn build_file_index(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
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
                files.push(path);
            }
        }
    }
    files
}

fn excluded_path(path: &Path, is_dir: bool) -> bool {
    if !is_dir {
        return false;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".venv"
                    | "venv"
                    | "__pycache__"
                    | ".cache"
                    | "Caches"
                    | "cache"
                    | "vendor"
                    | ".git"
            )
        })
}

/// Search the prepared application and file snapshots. The caller can run this
/// function on a worker thread; it performs no UI or blocking process work.
pub fn query(
    query: &str,
    apps: &[SearchResult],
    files: &[PathBuf],
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

    let mut ranked: Vec<(u8, i32, SearchResult)> = Vec::new();
    for app in apps {
        let score = best_score(
            needle,
            &[&app.title, &app.id, &app.subtitle, &app.search_terms],
        );
        if score > 0 {
            offer(&mut ranked, (0, score, app.clone()));
        }
    }
    for path in files {
        let display = path.to_string_lossy();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let score = best_score(needle, &[name.as_ref(), display.as_ref()]);
        if score > 0 {
            offer(
                &mut ranked,
                (
                    1,
                    score,
                    SearchResult {
                        id: format!("file:{}", path.display()),
                        title: name.into_owned(),
                        subtitle: display.into_owned(),
                        search_terms: String::new(),
                        icon: None,
                        kind: ResultKind::File { path: path.clone() },
                    },
                ),
            );
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

fn best_score(query: &str, candidates: &[&str]) -> i32 {
    candidates
        .iter()
        .map(|candidate| match_score(query, candidate))
        .max()
        .unwrap_or(0)
}

/// Deterministic case-insensitive fuzzy score: exact, prefix, word prefix,
/// contiguous substring, then ordered subsequence.
fn match_score(query: &str, candidate: &str) -> i32 {
    let q = query.to_lowercase();
    let c = candidate.to_lowercase();
    if q.is_empty() {
        return 0;
    }
    if c == q {
        return 10_000;
    }
    if c.starts_with(&q) {
        return 8_000 - c.len().min(1000) as i32;
    }
    if c.split(|ch: char| !ch.is_alphanumeric())
        .any(|word| word.starts_with(&q))
    {
        return 6_000 - c.len().min(1000) as i32;
    }
    if let Some(index) = c.find(&q) {
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

pub fn activate(result: &SearchResult) -> Result<(), String> {
    match &result.kind {
        ResultKind::App { desktop } => activate_app(desktop),
        ResultKind::File { path } => spawn_detached("xdg-open", &[path.as_os_str()]),
        ResultKind::Web { url } => spawn_detached("xdg-open", &[OsStr::new(url)]),
    }
}

fn activate_app(desktop: &Path) -> Result<(), String> {
    let locales = get_languages_from_env();
    let entry = DesktopEntry::from_path(desktop.to_path_buf(), Some(&locales))
        .map_err(|error| format!("Could not read application entry: {error}"))?;
    if entry.terminal() {
        let args = parse_exec(
            entry
                .exec()
                .ok_or_else(|| "Application has no Exec command".to_owned())?,
            entry.name(&locales).as_deref().unwrap_or(""),
            entry.icon().unwrap_or(""),
            desktop,
        )?;
        if args.is_empty() {
            return Err("Application has an empty Exec command".into());
        }
        let mut command = Command::new("cosmic-term");
        command.arg("-e").args(args);
        if let Some(path) = entry.path() {
            command.current_dir(path);
        }
        return spawn(command, "cosmic-term");
    }
    spawn_detached("gio", &[OsStr::new("launch"), desktop.as_os_str()])
}

fn spawn_detached(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    spawn(command, program)
}

fn spawn(mut command: Command, program: &str) -> Result<(), String> {
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start {program}: {error}"))?;
    // Reap the launcher helper without blocking the desktop UI or retaining zombies.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Parse the Desktop Entry Exec grammar's quoting and backslash rules, expanding
/// the file-independent codes safely without invoking a shell.
fn parse_exec(exec: &str, name: &str, icon: &str, desktop: &Path) -> Result<Vec<String>, String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut token = String::new();
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    let mut started = false;
    let mut omit_arg = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => {
                let next = chars
                    .next()
                    .ok_or_else(|| "Exec ends with an escape".to_owned())?;
                if matches!(next, '"' | '`' | '$' | '\\') || next.is_whitespace() {
                    token.push(next);
                } else {
                    token.push('\\');
                    token.push(next);
                }
                started = true;
            }
            '%' => {
                let code = chars
                    .next()
                    .ok_or_else(|| "Exec ends with a field code".to_owned())?;
                match code {
                    '%' => token.push('%'),
                    'f' | 'F' | 'u' | 'U' => {
                        omit_arg = true;
                    }
                    'i' => {
                        if !icon.is_empty() {
                            if started && !token.is_empty() {
                                return Err("Exec field %i must be a separate argument".into());
                            }
                            tokens.push("--icon".into());
                            tokens.push(icon.to_owned());
                            omit_arg = true;
                        } else {
                            omit_arg = true;
                        }
                    }
                    'c' => token.push_str(name),
                    'k' => token.push_str(&desktop.to_string_lossy()),
                    _ => return Err(format!("Unknown Exec field %{code}")),
                }
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started && !omit_arg {
                    tokens.push(std::mem::take(&mut token));
                }
                token.clear();
                started = false;
                omit_arg = false;
            }
            _ => {
                token.push(ch);
                started = true;
            }
        }
    }
    if quoted {
        return Err("Exec has an unmatched quote".into());
    }
    if started && !omit_arg {
        tokens.push(token);
    }
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

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
        let files = vec![PathBuf::from("/tmp/edit.txt"), PathBuf::from("/tmp/other")];
        let results = query("edit", &[app], &files, "duckduckgo");
        assert!(matches!(results[0].kind, ResultKind::App { .. }));
        assert!(matches!(results[1].kind, ResultKind::File { .. }));
        assert!(matches!(
            results.last().unwrap().kind,
            ResultKind::Web { .. }
        ));
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
    fn exec_parser_preserves_quoted_spaces_and_literal_percent() {
        assert_eq!(
            parse_exec(
                "cosmic-term --title \"hello world\" -e app %%",
                "App",
                "",
                Path::new("/tmp/app.desktop")
            )
            .unwrap(),
            vec!["cosmic-term", "--title", "hello world", "-e", "app", "%"]
        );
        assert_eq!(
            parse_exec("run %f file", "App", "", Path::new("/tmp/app.desktop")).unwrap(),
            vec!["run", "file"]
        );
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
        symlink(outside.path(), root.path().join("linked-dir")).unwrap();
        let index = build_file_index(&[root.path().to_path_buf()]);
        assert_eq!(index, vec![root.path().join("src/main.rs")]);
    }

    #[test]
    fn desktop_user_override_wins_even_when_hidden() {
        let user = tempfile::tempdir().unwrap();
        let system = tempfile::tempdir().unwrap();
        let user_path = user.path().join("example.desktop");
        let system_path = system.path().join("example.desktop");
        fs::write(
            &user_path,
            "[Desktop Entry]\nType=Application\nName=User\nExec=true\nHidden=true\n",
        )
        .unwrap();
        fs::write(
            &system_path,
            "[Desktop Entry]\nType=Application\nName=System\nExec=true\n",
        )
        .unwrap();
        let entries =
            Iter::new(vec![user.path().to_path_buf(), system.path().to_path_buf()].into_iter())
                .entries(Some(&[] as &[String]))
                .collect::<Vec<_>>();
        let mut winners = HashMap::new();
        for entry in entries {
            winners.entry(entry.id().to_owned()).or_insert(entry);
        }
        assert!(
            winners
                .values()
                .any(|entry| entry.path == user_path && entry.hidden())
        );
        assert!(!winners.values().any(|entry| entry.path == system_path));
    }

    #[test]
    fn localized_app_name_and_desktop_visibility_are_respected() {
        let entry = DesktopEntry::from_str(
            "/tmp/org.example.Locale.desktop",
            "[Desktop Entry]\nType=Application\nName=Default\nName[fr]=Traduit\nExec=true\nOnlyShowIn=GNOME;COSMIC;\nNotShowIn=KDE;\n",
            Some(&["fr".to_owned()]),
        ).unwrap();
        assert_eq!(entry.name(&["fr"]).unwrap(), "Traduit");
        assert!(visible_on_this_desktop(&entry, &["cosmic".into()]));
        assert!(!visible_on_this_desktop(&entry, &["kde".into()]));
        assert!(!visible_on_this_desktop(&entry, &[]));
    }
}
