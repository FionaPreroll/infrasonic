// SPDX-License-Identifier: GPL-3.0-only
//! Configuration on the command line: loading, STMPS import, adding profiles.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use infrasonic_core::config::{ConfigError, profile_from_url, resolve_auth};
use infrasonic_core::{Config, Paths, Profile};
use infrasonic_subsonic::Client;

use crate::cli::Args;

/// The configuration file to use.
pub fn config_path(args: &Args) -> PathBuf {
    args.config
        .clone()
        .unwrap_or_else(|| Paths::platform().config_file)
}

/// Load the configuration. On the very first start an STMPS configuration is imported.
pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    if !path.exists()
        && let Some(stmps) = Paths::stmps_candidates().into_iter().find(|p| p.exists())
    {
        let c = import_stmps_file(&stmps)?;
        c.save(path)?;
        eprintln!(
            "Imported the STMPS configuration from {} into {}",
            stmps.display(),
            path.display()
        );
        return Ok(c);
    }
    Ok(Config::load(path)?)
}

fn import_stmps_file(path: &Path) -> anyhow::Result<Config> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Config::from_stmps(&text, path)?)
}

/// `--import-stmps [FILE]`: merge the STMPS profile into the configuration.
pub fn import_stmps(arg: Option<&Path>, config_file: &Path) -> anyhow::Result<()> {
    let source = match arg {
        Some(p) => p.to_path_buf(),
        None => Paths::stmps_candidates()
            .into_iter()
            .find(|p| p.exists())
            .context("no stmp.toml found; pass its path")?,
    };
    let imported = import_stmps_file(&source)?;
    let existed = config_file.exists();
    let mut config = Config::load(config_file)?;
    for (name, p) in imported.profiles {
        let mut name = name;
        while config.profiles.contains_key(&name) {
            name.push_str("-stmps");
        }
        println!("Imported profile `{name}` ({})", p.url);
        config.default_profile.get_or_insert_with(|| name.clone());
        config.profiles.insert(name, p);
    }
    if !existed {
        config.playback = imported.playback;
        config.ui = imported.ui;
    }
    config.save(config_file)?;
    println!("Wrote {}", config_file.display());
    Ok(())
}

/// `--list`.
pub fn list_profiles(config: &Config, path: &Path) {
    if config.profiles.is_empty() {
        println!("No profiles in {}", path.display());
        return;
    }
    for (name, p) in &config.profiles {
        let default = if config.default_profile.as_deref() == Some(name.as_str())
            || config.profiles.len() == 1
        {
            " (default)"
        } else {
            ""
        };
        println!("{name}{default}: {} as {}", p.url, p.username);
    }
}

/// The profile to connect with.
pub fn select_profile(
    args: &Args,
    config: &mut Config,
    path: &Path,
) -> anyhow::Result<(String, Profile)> {
    if let Some(url) = &args.url {
        return Ok(("command-line".into(), profile_from_url(url)?));
    }
    match config.profile(args.profile.as_deref()) {
        Ok(p) => Ok(p),
        Err(ConfigError::NoProfile(_)) if config.profiles.is_empty() && is_interactive() => {
            println!("No server configured yet.");
            add_profile(config, path)
        }
        Err(ConfigError::NoProfile(_)) if config.profiles.is_empty() => {
            Err(ConfigError::NoProfile(path.to_path_buf()).into())
        }
        Err(ConfigError::NoProfile(_)) => bail!(
            "several profiles are configured; choose one with --profile or set default_profile in {}",
            path.display()
        ),
        Err(e) => Err(e.into()),
    }
}

fn is_interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

fn ask(prompt: &str, default: Option<&str>) -> anyhow::Result<String> {
    match default {
        Some(d) => print!("{prompt} [{d}]: "),
        None => print!("{prompt}: "),
    }
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim().to_string();
    Ok(match (line.is_empty(), default) {
        (true, Some(d)) => d.to_string(),
        _ => line,
    })
}

/// Read a line without echo.
fn ask_hidden(prompt: &str) -> anyhow::Result<String> {
    use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, read};
    print!("{prompt}: ");
    std::io::stdout().flush()?;
    crossterm::terminal::enable_raw_mode()?;
    let mut s = String::new();
    let result = loop {
        match read() {
            Ok(Event::Key(k)) if k.kind != KeyEventKind::Release => match k.code {
                KeyCode::Enter => break Ok(()),
                KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(anyhow::anyhow!("cancelled"));
                }
                KeyCode::Backspace => {
                    s.pop();
                }
                KeyCode::Char(c) => s.push(c),
                _ => {}
            },
            Ok(_) => {}
            Err(e) => break Err(e.into()),
        }
    };
    crossterm::terminal::disable_raw_mode()?;
    println!();
    result.map(|()| s)
}

/// Ask for a server, test it and save it as a profile.
pub fn add_profile(config: &mut Config, path: &Path) -> anyhow::Result<(String, Profile)> {
    if !is_interactive() {
        bail!("adding a profile needs a terminal");
    }
    let name = ask("Profile name", Some("default"))?;
    let url = ask("Server URL (e.g. https://music.example.org)", None)?;
    let mut profile = profile_from_url(&url)?;
    if profile.username.is_empty() {
        profile.username = ask("User name", None)?;
    }
    let password = match profile.password.take() {
        Some(p) => p,
        None => ask_hidden("Password")?,
    };
    profile.password = Some(password.clone());
    print!("Connecting… ");
    std::io::stdout().flush()?;
    test_connection(&name, &profile)?;
    println!("ok");
    profile.password = None;
    match store_in_keyring(&profile, &password) {
        Ok(()) => println!("Stored the password in the system keyring."),
        Err(e) => {
            println!("The system keyring is not available ({e}).");
            let a = ask(
                "Store the password in the configuration file instead? (y/N)",
                None,
            )?;
            if a.eq_ignore_ascii_case("y") {
                profile.password = Some(password.clone());
            } else {
                println!(
                    "Set `password_command` in {} or INFRASONIC_PASSWORD.",
                    path.display()
                );
            }
        }
    }
    config.profiles.insert(name.clone(), profile.clone());
    config.default_profile.get_or_insert_with(|| name.clone());
    config.save(path)?;
    println!("Saved profile `{name}` in {}", path.display());
    if profile.password.is_none() {
        // The keyring (or nothing) holds it now; keep it for this run.
        profile.password = Some(password);
    }
    Ok((name, profile))
}

fn test_connection(name: &str, profile: &Profile) -> anyhow::Result<()> {
    let auth = resolve_auth(name, profile)?;
    let client = Client::new(&profile.url, auth)?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(client.ping())?;
    Ok(())
}

#[cfg(feature = "keyring")]
fn store_in_keyring(profile: &Profile, password: &str) -> Result<(), String> {
    infrasonic_core::config::store_password(profile, password)
}

#[cfg(not(feature = "keyring"))]
fn store_in_keyring(_: &Profile, _: &str) -> Result<(), String> {
    Err("built without keyring support".into())
}

/// `--store-password PROFILE`.
pub fn store_password(config: &Config, name: &str) -> anyhow::Result<()> {
    let (_, profile) = config.profile(Some(name))?;
    let pw = ask_hidden(&format!(
        "Password for {} at {}",
        profile.username, profile.url
    ))?;
    store_in_keyring(&profile, &pw).map_err(|e| anyhow::anyhow!(e))?;
    println!("Stored. You can remove `password` from the configuration file.");
    Ok(())
}
