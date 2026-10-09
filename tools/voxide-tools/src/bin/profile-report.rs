//! Summarise a PSoXide `--profile-log` CSV in VoXide's own stage names.

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        print!("{}", voxide_tools::profile::USAGE);
        std::process::exit(2);
    };
    let csv = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("profile-report: {path}: {e}");
            std::process::exit(1);
        }
    };
    match voxide_tools::profile::report(&csv) {
        Ok((status, text)) => {
            print!("{text}");
            std::process::exit(status);
        }
        Err(e) => {
            eprintln!("profile-report: {e}");
            std::process::exit(1);
        }
    }
}
