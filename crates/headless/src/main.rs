// SPDX-License-Identifier: GPL-3.0-only
//! `infrasonic-headless`: play without a user interface.

use std::process::ExitCode;

use clap::Parser;
use infrasonic_engine::output::{CpalOutputFactory, NullOutputFactory, OutputFactory, OutputSpec};
use infrasonic_headless::{Args, run};

fn main() -> ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("INFRASONIC_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();
    let output: Box<dyn OutputFactory> = if args.null {
        let spec = OutputSpec {
            sample_rate: 48_000,
            channels: 2,
        };
        Box::new(NullOutputFactory::new(spec, 1.0))
    } else {
        Box::new(CpalOutputFactory { device: None })
    };
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("infrasonic-headless: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(run(&args, output)) {
        Ok(r) if r.errors.is_empty() => ExitCode::SUCCESS,
        Ok(r) => {
            eprintln!("infrasonic-headless: {} errors", r.errors.len());
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("infrasonic-headless: {e:#}");
            ExitCode::FAILURE
        }
    }
}
