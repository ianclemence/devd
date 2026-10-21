// A stand-in C compiler for `cargo check` across targets.
//
// It accepts any command line and writes an empty file wherever an output is
// named (`-o file`, `-ofile`, `-Fofile`, `/Fo file`). Nothing that uses it is
// ever linked or run; it only lets build scripts that compile C get out of the
// way so that the Rust code can be type-checked for another operating system.
// See scripts/cross-check.ps1.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if (arg == "-o" || arg == "/Fo" || arg == "-Fo") && i + 1 < args.len() {
            let _ = std::fs::write(&args[i + 1], b"");
        } else if let Some(path) = arg
            .strip_prefix("-Fo")
            .or_else(|| arg.strip_prefix("/Fo"))
            .or_else(|| arg.strip_prefix("-o"))
        {
            if !path.is_empty() {
                let _ = std::fs::write(path, b"");
            }
        }
        i += 1;
    }
}
