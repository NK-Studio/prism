#![no_main]

use libfuzzer_sys::fuzz_target;

// The whole native reader (archive, object stream, model) and the preview, on arbitrary bytes.
fuzz_target!(|data: &[u8]| {
    let _ = prism_affinity::container::header(data);
    let _ = prism_affinity::preview(data);
    let _ = prism_affinity::read(data, prism_affinity::Limits { max_entry: 16 << 20, max_total: 64 << 20 });
});
