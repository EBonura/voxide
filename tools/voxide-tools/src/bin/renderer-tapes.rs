//! Generate the frozen standing and walking/mining renderer benchmark tapes.
//!
//! Usage: renderer-tapes --out DIR

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = args.next().map(PathBuf::from),
            "-h" | "--help" => {
                println!("Usage: renderer-tapes --out DIR");
                return;
            }
            other => {
                eprintln!("renderer-tapes: unexpected argument {other:?}");
                std::process::exit(2);
            }
        }
    }
    let Some(out) = out else {
        eprintln!("renderer-tapes: --out DIR is required");
        std::process::exit(2);
    };
    match voxide_tools::tapes::run(&out) {
        Ok(paths) => {
            for path in paths {
                println!("{}", path.display());
            }
        }
        Err(e) => {
            eprintln!("renderer-tapes: {e}");
            std::process::exit(1);
        }
    }
}
