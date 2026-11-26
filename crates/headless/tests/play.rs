// SPDX-License-Identifier: GPL-3.0-only
//! The headless player with local files, and against Navidrome and gonic when
//! `scripts/e2e-servers.sh` runs them.

use std::path::Path;

use clap::Parser;
use infrasonic_e2e::{null_output, servers};
use infrasonic_headless::{Args, run};

fn args(extra: &[&str]) -> Args {
    let mut v = vec!["infrasonic-headless", "--status-every", "100"];
    v.extend_from_slice(extra);
    Args::parse_from(v)
}

#[tokio::test(flavor = "multi_thread")]
async fn plays_local_files() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/gapless");
    let files: Vec<String> = ["flac/part1.flac", "opus/part2.opus", "mp3/part3.mp3"]
        .iter()
        .map(|f| dir.join(f).to_string_lossy().into_owned())
        .collect();
    let mut a: Vec<&str> = files.iter().map(String::as_str).collect();
    a.push("--replay-gain=track");
    let r = run(&args(&a), null_output(20.0)).await.expect("run");
    assert_eq!(r.played, ["part1", "part2", "part3"]);
    assert!(r.finished && r.errors.is_empty(), "{r:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_file_is_an_error() {
    let e = run(&args(&["/nonexistent.flac"]), null_output(1.0))
        .await
        .expect_err("missing");
    assert!(e.to_string().contains("not a file"), "{e}");
}

#[tokio::test(flavor = "multi_thread")]
async fn plays_from_servers() {
    for s in servers() {
        let url = s.url.replacen("://", "://admin:admin@", 1);
        let r = run(
            &args(&["--url", &url, "--search", "song", "--bitrate", "0"]),
            null_output(20.0),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e:#}", s.name));
        assert!(r.played.len() >= 3, "{}: {r:?}", s.name);
        assert!(r.finished && r.errors.is_empty(), "{}: {r:?}", s.name);

        let r = run(
            &args(&["--url", &url, "--random", "2", "--seconds", "1"]),
            null_output(1.0),
        )
        .await
        .unwrap_or_else(|e| panic!("{}: {e:#}", s.name));
        assert!(!r.played.is_empty() && !r.finished, "{}: {r:?}", s.name);
    }
}
