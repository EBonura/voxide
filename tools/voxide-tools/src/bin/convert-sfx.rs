//! Cook the sound effects into SPU-ADPCM: the bank and `game/src/sfxdata.rs`.

fn main() {
    if let Err(e) = voxide_tools::sfx::run(&voxide_tools::repo_root()) {
        eprintln!("convert-sfx: {e}");
        std::process::exit(1);
    }
}
