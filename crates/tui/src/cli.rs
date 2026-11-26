// SPDX-License-Identifier: GPL-3.0-only
//! Command line arguments.

use std::path::PathBuf;

use clap::Parser;

/// A terminal music player for Subsonic and OpenSubsonic servers.
#[derive(Debug, Clone, Parser)]
#[command(name = "infrasonic", version, about, long_about = None)]
pub struct Args {
    /// Server to connect to, like `https://user:password@music.example.org` (STMPS syntax).
    /// Overrides the profile.
    pub url: Option<String>,

    /// Configuration file.
    #[arg(short, long, env = "INFRASONIC_CONFIG", value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Server profile from the configuration file.
    #[arg(short, long, env = "INFRASONIC_PROFILE")]
    pub profile: Option<String>,

    /// List the configured profiles and exit.
    #[arg(long)]
    pub list: bool,

    /// List the audio output devices and exit.
    #[arg(long)]
    pub list_devices: bool,

    /// Import an STMPS `stmp.toml` (found automatically if no file is given), then exit.
    #[arg(long, value_name = "FILE")]
    pub import_stmps: Option<Option<PathBuf>>,

    /// Add a server profile interactively, then exit.
    #[arg(long)]
    pub add_profile: bool,

    /// Store the password of a profile in the system keyring, then exit.
    #[arg(long, value_name = "PROFILE")]
    pub store_password: Option<String>,

    /// Browse by tags (ID3) instead of folders.
    #[arg(long)]
    pub tags: bool,

    /// Start without contacting the server and play pinned songs only.
    #[arg(long)]
    pub offline: bool,

    /// Log file (default: platform state directory).
    #[arg(long, value_name = "FILE")]
    pub log_file: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_is_consistent() {
        Args::command().debug_assert();
    }

    #[test]
    fn import_without_file() {
        let a = Args::parse_from(["infrasonic", "--import-stmps"]);
        assert_eq!(a.import_stmps, Some(None));
        let a = Args::parse_from(["infrasonic", "--import-stmps", "x.toml"]);
        assert_eq!(a.import_stmps, Some(Some(PathBuf::from("x.toml"))));
        let a = Args::parse_from(["infrasonic", "https://u:p@h", "-p", "x"]);
        assert_eq!(a.url.as_deref(), Some("https://u:p@h"));
        assert_eq!(a.profile.as_deref(), Some("x"));
    }
}
