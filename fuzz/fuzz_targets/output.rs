//! The `output` fuzz target; see fuzz/README.md.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| jev_cli::fuzz::output(data));
