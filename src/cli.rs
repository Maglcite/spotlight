//! Command-line arguments. The launcher flags map onto [`ipc::Command`] so a
//! second invocation can be forwarded to the running instance.

use crate::ipc;
use clap::{Parser, Subcommand};

#[derive(Parser, Clone, Debug)]
#[command(
    name = "spotlight",
    version,
    about = "A translucent launcher and read-only AI assistant"
)]
pub struct Args {
    #[arg(long, help = "Start in the background without opening the launcher")]
    pub daemon: bool,
    #[arg(long, help = "Toggle the launcher (the default action)")]
    pub toggle: bool,
    #[arg(long, help = "Open AI and search settings")]
    pub settings: bool,
    #[arg(
        long,
        value_name = "TEXT",
        help = "Open the launcher with a search query"
    )]
    pub query: Option<String>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Headless subcommands that exercise the core without a UI.
#[derive(Subcommand, Clone, Debug)]
pub enum Command {
    /// Print ranked results (apps, files, web fallback) for a query.
    Search {
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Skip building the file index (apps and web results only).
        #[arg(long)]
        no_files: bool,
        /// Activate the top result after printing.
        #[arg(long)]
        open: bool,
    },
    /// Ask the configured AI provider and stream the answer to stdout.
    Ask {
        #[arg(required = true, num_args = 1.., value_name = "PROMPT")]
        prompt: Vec<String>,
    },
}

impl Args {
    /// The launcher action requested by the flags. `None` means `--daemon`:
    /// start (or stay) in the background without showing anything.
    pub fn ipc_command(&self) -> Option<ipc::Command> {
        if self.settings {
            Some(ipc::Command::Settings)
        } else if let Some(query) = &self.query {
            Some(ipc::Command::Query(query.clone()))
        } else if self.daemon {
            None
        } else {
            Some(ipc::Command::Toggle)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_map_to_launcher_commands() {
        let parse = |args: &[&str]| Args::try_parse_from(args).unwrap();
        assert_eq!(
            parse(&["spotlight"]).ipc_command(),
            Some(ipc::Command::Toggle)
        );
        assert_eq!(parse(&["spotlight", "--daemon"]).ipc_command(), None);
        assert_eq!(
            parse(&["spotlight", "--query", "fire"]).ipc_command(),
            Some(ipc::Command::Query("fire".into()))
        );
        let search = parse(&["spotlight", "search", "two", "words"]);
        assert!(
            matches!(search.command, Some(Command::Search { ref query, .. }) if query.join(" ") == "two words")
        );
    }
}
