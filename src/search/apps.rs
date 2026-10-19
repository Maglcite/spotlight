//! Installed application discovery: freedesktop `.desktop` entries on Linux and
//! Start Menu shortcuts on Windows.

use super::{ResultKind, SearchResult};
#[cfg(any(windows, test))]
use std::collections::HashSet;
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};

/// Discover installed applications, sorted by title.
pub fn discover_apps() -> Vec<SearchResult> {
    #[cfg(target_os = "linux")]
    {
        linux::discover()
    }
    #[cfg(windows)]
    {
        windows::discover()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

fn sort_apps(apps: &mut [SearchResult]) {
    apps.sort_by(|a, b| {
        a.title
            .to_lowercase()
            .cmp(&b.title.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{ResultKind, SearchResult, sort_apps};
    use freedesktop_desktop_entry::{
        DesktopEntry, Iter, current_desktop, default_paths, get_languages_from_env,
    };
    use std::collections::{HashMap, HashSet};
    use std::env;
    use std::path::PathBuf;

    /// Discover installed desktop applications. Search roots are kept in XDG priority
    /// order so a user's entry shadows a system entry with the same desktop ID.
    pub(super) fn discover() -> Vec<SearchResult> {
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
                if title.trim().is_empty()
                    || entry.type_().is_some_and(|kind| kind != "Application")
                {
                    return None;
                }
                if let Some(try_exec) = entry.try_exec()
                    && !crate::launch::executable_exists(try_exec)
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
        sort_apps(&mut apps);
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

    pub(super) fn visible_on_this_desktop(entry: &DesktopEntry, desktops: &[String]) -> bool {
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;

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
}

#[cfg(windows)]
mod windows {
    use super::{SearchResult, scan_shortcuts};
    use std::path::PathBuf;

    /// Per-user Start Menu first so a user's shortcut shadows the machine-wide one.
    /// Store/UWP apps (`shell:AppsFolder`) are not enumerated yet.
    pub(super) fn discover() -> Vec<SearchResult> {
        let programs = PathBuf::from("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs");
        let roots = [
            std::env::var_os("APPDATA").map(PathBuf::from),
            std::env::var_os("ProgramData").map(PathBuf::from),
        ]
        .into_iter()
        .flatten()
        .map(|base| base.join(&programs))
        .collect::<Vec<_>>();
        scan_shortcuts(&roots)
    }
}

#[cfg(any(windows, test))]
const SHORTCUT_EXTENSIONS: &[&str] = &["lnk", "url", "appref-ms"];
#[cfg(any(windows, test))]
const MAX_SHORTCUT_DEPTH: usize = 6;

/// Collect launchable shortcuts below `roots`. Earlier roots win when two
/// shortcuts share a (case-insensitive) title.
#[cfg(any(windows, test))]
fn scan_shortcuts(roots: &[PathBuf]) -> Vec<SearchResult> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for root in roots {
        let mut found = Vec::new();
        collect_shortcuts(root, 0, &mut found);
        // Directory order is filesystem-dependent; keep shadowing deterministic.
        found.sort();
        for path in found {
            let Some(title) = path.file_stem().and_then(|s| s.to_str()).map(str::trim) else {
                continue;
            };
            if title.is_empty() || !useful_shortcut(title) || !seen.insert(title.to_lowercase()) {
                continue;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let folder = relative
                .parent()
                .map(|p| p.display().to_string())
                .filter(|p| !p.is_empty());
            apps.push(SearchResult {
                id: relative.display().to_string(),
                title: title.to_owned(),
                subtitle: folder.clone().unwrap_or_else(|| "Application".into()),
                search_terms: folder.unwrap_or_default(),
                icon: None,
                kind: ResultKind::App { desktop: path },
            });
        }
    }
    sort_apps(&mut apps);
    apps
}

#[cfg(any(windows, test))]
fn collect_shortcuts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_SHORTCUT_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            collect_shortcuts(&path, depth + 1, out);
        } else if kind.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|ext| {
                    SHORTCUT_EXTENSIONS
                        .iter()
                        .any(|x| x.eq_ignore_ascii_case(ext))
                })
        {
            out.push(path);
        }
    }
}

/// Skip uninstallers and documentation links that installers drop next to apps.
#[cfg(any(windows, test))]
fn useful_shortcut(title: &str) -> bool {
    let lower = title.to_lowercase();
    !(lower.contains("uninstall")
        || lower.contains("readme")
        || lower.contains("read me")
        || lower.contains("documentation")
        || lower.contains("release notes")
        || lower.contains("license")
        || lower == "help"
        || lower.ends_with(" help")
        || lower.ends_with(" manual")
        || lower.ends_with(" website"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn start_menu_scan_filters_and_dedupes_shortcuts() {
        let user = tempfile::tempdir().unwrap();
        let machine = tempfile::tempdir().unwrap();
        fs::create_dir_all(user.path().join("Tools")).unwrap();
        fs::write(user.path().join("Tools").join("Editor.lnk"), "").unwrap();
        fs::write(user.path().join("Uninstall Editor.lnk"), "").unwrap();
        fs::write(user.path().join("Editor Help.lnk"), "").unwrap();
        fs::write(user.path().join("desktop.ini"), "").unwrap();
        fs::write(machine.path().join("editor.lnk"), "").unwrap();
        fs::write(machine.path().join("Docs Site.url"), "").unwrap();
        fs::write(machine.path().join("Click Once.appref-ms"), "").unwrap();
        let apps = scan_shortcuts(&[user.path().to_path_buf(), machine.path().to_path_buf()]);
        let titles = apps.iter().map(|a| a.title.as_str()).collect::<Vec<_>>();
        assert_eq!(titles, ["Click Once", "Docs Site", "Editor"]);
        let editor = apps.iter().find(|a| a.title == "Editor").unwrap();
        assert_eq!(editor.subtitle, "Tools");
        assert!(
            matches!(&editor.kind, ResultKind::App { desktop } if desktop.starts_with(user.path()))
        );
    }
}
