// SPDX-License-Identifier: GPL-3.0-only
//! A MilkDrop preset as the visualizer reads one from the downloaded collection, through Lactea:
//! the file, every expression compiled and run once within the budgets of
//! `infrasonic_visualizer::Limits::phone()`, and both shaders translated to WGSL. No panics, no
//! stack overflow, no loop or allocation beyond the budgets.
#![no_main]

use std::sync::Arc;

use lactea_abi::PassKind;
use lactea_eel::{Context, GlobalMemory};
use lactea_preset::{Preset, code_to_source};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(preset) = Preset::parse(data) else {
        return;
    };
    let memory = Arc::new(GlobalMemory::new());
    memory.set_loop_budget(Some(1 << 20));
    memory.set_memory_budget(Some(32 << 20));
    let waves = preset.waves.iter().flat_map(|w| [&w.init, &w.per_frame, &w.per_point]);
    let shapes = preset.shapes.iter().flat_map(|s| [&s.init, &s.per_frame]);
    let main = [&preset.per_frame_init, &preset.per_frame, &preset.per_pixel];
    for code in main.into_iter().chain(waves).chain(shapes) {
        let mut ctx = Context::new(Arc::clone(&memory));
        if let Ok(program) = ctx.compile(&code_to_source(code)) {
            ctx.run(&program);
        }
    }
    let mut opts = lactea_hlsl::Options::default();
    opts.max_loop_iterations = Some(4096);
    for (kind, src) in [
        (PassKind::Warp, &preset.warp_shader),
        (PassKind::Comp, &preset.comp_shader),
    ] {
        if let Some(src) = src {
            let _ = lactea_hlsl::translate(kind, src, &opts);
        }
    }
});
