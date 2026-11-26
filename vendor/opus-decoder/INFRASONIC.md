# Patched opus-decoder 0.1.1

Copy of [opus-decoder 0.1.1](https://crates.io/crates/opus-decoder)
(<https://github.com/TadeuszWolfGang/Rusopus>, MIT OR Apache-2.0) with two fixes, used through
`[patch.crates-io]` in the workspace `Cargo.toml`. Remove this directory and the patch entry
once a release upstream contains equivalent fixes.

1. `src/celt/kiss_fft.rs`, `flat_fft_forward`: the O(n²) DFT called `sin_cos` for every term.
   Precomputing the n twiddle factors per call makes decoding about 7× faster
   (5 s of 48 kHz stereo: 1.9 s → 0.26 s on x86-64, release build). A real FFT would be faster
   still.
2. `src/celt/vq.rs`, `extract_collapse_mask`: `1u8 << i` overflowed for `i >= 8` on malformed
   input (found by `fuzz/` target `decode`). Bits beyond the eighth are now dropped, as in
   libopus where the mask is truncated to `unsigned char`.

Only `src/`, `Cargo.toml` and `README.md` are copied; the crate's tests live upstream.
