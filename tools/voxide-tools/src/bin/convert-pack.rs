//! Quantise the CC0 block pack into `game/src/texdata.rs`.

fn main() {
    if let Err(e) = voxide_tools::pack::run(&voxide_tools::repo_root()) {
        eprintln!("convert-pack: {e}");
        std::process::exit(1);
    }
}
