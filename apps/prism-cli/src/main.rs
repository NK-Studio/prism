#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "linux")]
compile_error!("Prism no longer supports Linux application builds.");

fn main() {
    // `args_os`, not `args`: a non-Unicode argument (a non-Unicode Unix file name) must be a
    // usage error, not a panic before anything is printed (issue #1108).
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let code = prism_cli::run_os(&args, &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
