//! Cook the songs in `assets/music/tracks.txt` into `MUSIC<n>.XA` and
//! `game/src/musicdata.rs`.
//!
//! `convert-music --src DIR --cook PATH` (see voxide_tools::music).

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = voxide_tools::repo_root();
    let result = voxide_tools::music::job_from_args(&root, &args)
        .and_then(|job| voxide_tools::music::run(&job));
    if let Err(e) = result {
        eprintln!("convert-music: {e}");
        std::process::exit(1);
    }
}
