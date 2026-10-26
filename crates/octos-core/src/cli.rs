//! Command-line subcommands.
//!
//! This mirrors the original `src/CLI/CLI.cpp` parsing rules: only the first
//! non-flag argument is a path, `--auto` is accepted and ignored, and an
//! unknown subcommand does nothing. Path resolution and existence checks are
//! left to the caller so this module stays testable off Windows.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Copy a folder into the wallpapers directory and show it on every monitor.
    Run { folder: PathBuf, dev_tools: bool },
    /// Reload every wallpaper WebView.
    Reload,
    /// Create a starter mod (`octos.json` + `index.html`) in a folder.
    New { folder: PathBuf },
    /// Open DevTools on every wallpaper WebView.
    DevTools,
    /// Download and install a wallpaper from the community catalog.
    CommunityInstall { folder: String },
    /// Refresh the community catalog from its source.
    CommunityRefresh,
    /// Cancel an in-flight community download.
    CommunityCancel,
}

/// Parse arguments *after* the executable name.
///
/// Returns `None` when no subcommand was given (the caller restores the main
/// window) or when the subcommand is unknown, matching the old app.
pub fn parse(args: &[String]) -> Option<Command> {
    let subcommand = args.first()?;
    match subcommand.as_str() {
        "run" => {
            let mut dev_tools = false;
            let mut folder = None;
            for arg in &args[1..] {
                if arg == "--dev-tools" {
                    dev_tools = true;
                } else if arg == "--auto" {
                    // Accepted for compatibility; the old app never used it.
                } else if !arg.is_empty() && !arg.starts_with('-') && folder.is_none() {
                    folder = Some(PathBuf::from(arg));
                }
            }
            Some(Command::Run {
                folder: folder.unwrap_or_else(|| PathBuf::from(".")),
                dev_tools,
            })
        }
        "reload" => Some(Command::Reload),
        "new" => {
            let folder = args[1..]
                .iter()
                .find(|arg| !arg.is_empty() && !arg.starts_with('-'))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("new-mod"));
            Some(Command::New { folder })
        }
        "dev-tools" => Some(Command::DevTools),
        "community" => match args.get(1).map(String::as_str) {
            Some("install") => args
                .get(2)
                .filter(|folder| !folder.is_empty() && !folder.starts_with('-'))
                .map(|folder| Command::CommunityInstall {
                    folder: folder.clone(),
                }),
            Some("refresh") => Some(Command::CommunityRefresh),
            Some("cancel") => Some(Command::CommunityCancel),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn no_arguments_means_no_command() {
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn community_subcommands() {
        assert_eq!(
            parse(&args(&["community", "install", "2961764492"])),
            Some(Command::CommunityInstall {
                folder: "2961764492".to_string()
            })
        );
        assert_eq!(parse(&args(&["community", "install"])), None);
        assert_eq!(
            parse(&args(&["community", "refresh"])),
            Some(Command::CommunityRefresh)
        );
        assert_eq!(
            parse(&args(&["community", "cancel"])),
            Some(Command::CommunityCancel)
        );
        assert_eq!(parse(&args(&["community", "nope"])), None);
    }

    #[test]
    fn run_takes_first_path_and_flags_in_any_order() {
        assert_eq!(
            parse(&args(&["run", "--dev-tools", "C:\\mods\\one", "--auto"])),
            Some(Command::Run {
                folder: PathBuf::from("C:\\mods\\one"),
                dev_tools: true,
            })
        );
        assert_eq!(
            parse(&args(&["run", "one", "two"])),
            Some(Command::Run {
                folder: PathBuf::from("one"),
                dev_tools: false,
            })
        );
    }

    #[test]
    fn run_without_folder_uses_the_current_directory() {
        assert_eq!(
            parse(&args(&["run"])),
            Some(Command::Run {
                folder: PathBuf::from("."),
                dev_tools: false,
            })
        );
    }

    #[test]
    fn new_defaults_to_new_mod() {
        assert_eq!(
            parse(&args(&["new"])),
            Some(Command::New {
                folder: PathBuf::from("new-mod"),
            })
        );
        assert_eq!(
            parse(&args(&["new", "--ignored", "my-mod"])),
            Some(Command::New {
                folder: PathBuf::from("my-mod"),
            })
        );
    }

    #[test]
    fn simple_commands_and_unknown_subcommands() {
        assert_eq!(parse(&args(&["reload"])), Some(Command::Reload));
        assert_eq!(parse(&args(&["dev-tools"])), Some(Command::DevTools));
        assert_eq!(parse(&args(&["does-not-exist"])), None);
    }
}
