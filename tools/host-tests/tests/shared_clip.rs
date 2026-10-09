//! Check the actual production clipping adapter against the frozen renderer
//! oracle.

mod common;

/// The text of the item starting at `signature`, through its matching brace.
fn item<'a>(text: &'a str, signature: &str) -> &'a str {
    let start = text
        .find(signature)
        .unwrap_or_else(|| panic!("{signature:?} not found"));
    let open = start + text[start..].find('{').expect("opening brace");
    let mut depth = 1;
    let mut end = open + 1;
    for (offset, c) in text[open + 1..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            end = open + 1 + offset + 1;
            break;
        }
    }
    &text[start..end]
}

const MAIN: &str = r####"
fn check<const P:usize>(input:&[ClipVert;12],n:usize)->u64 {
 let mut a=[EMPTY_CLIP_VERT;12];let mut b=a;
 let na=legacy_clip::<P>(input,n,&mut a);let nb=clip_polygon_plane_c::<P>(input,n,&mut b);
 assert_eq!(na,nb);assert_eq!(&a[..na],&b[..nb]);
 let mut h=0xcbf29ce484222325u64;
 for v in &a[..na] { for x in [v.x,v.y,v.z,v.u,v.v,v.r,v.g,v.b] {h=(h^(x as u32 as u64)).wrapping_mul(0x100000001b3);} }
 h
}
fn main(){
 let mut seed=0x12345678u32;let mut hash=0u64;let mut tests=0;
 for _ in 0..50000 {
  let mut next=||{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);seed};
  let x=(next()%1800)as i32-900;let y=(next()%1400)as i32-700;let z=(next()%1400)as i32-100;
  let dx=(next()%400+1)as i32;let dy=(next()%400+1)as i32;
  let mut a=[EMPTY_CLIP_VERT;12];
  for (i,(vx,vy)) in [(x,y),(x+dx,y),(x+dx,y+dy),(x,y+dy)].into_iter().enumerate(){a[i]=ClipVert{x:vx,y:vy,z,u:(i as i32)*4096,v:(3-i as i32)*4096,r:(next()%256)as i32,g:(next()%256)as i32,b:(next()%256)as i32};}
  hash^=check::<0>(&a,4);hash^=check::<1>(&a,4);hash^=check::<2>(&a,4);hash^=check::<3>(&a,4);hash^=check::<4>(&a,4);hash^=check::<5>(&a,4);tests+=6;
 }
 println!("{tests} actual-adapter cases exact; fingerprint {hash:016x}");
}
"####;

#[test]
fn actual_adapter_preserves_vertices_attributes_and_order() {
    let root = common::root();
    let source = std::fs::read_to_string(root.join("game/src/main.rs")).expect("game/src/main.rs");
    let kernel = root.join(".psoxide/sdk/crates/psx-math/src");
    assert!(
        kernel.is_dir(),
        "{} is missing: run `make psoxide` to import the locked SDK",
        kernel.display()
    );
    let mut code = String::from("#![allow(dead_code)]\n");
    code += &format!("#[path=\"{}\"] mod int32;\n", kernel.join("int32.rs").display());
    code += &format!(
        "#[path=\"{}\"] mod attributed_clip;\n",
        kernel.join("attributed_clip.rs").display()
    );
    code += "use attributed_clip::{AttributedClipPlane,ClipTraversal,clip_convex_plane};\n";
    code += "const NEAR_Z:i32=18;const FAR_Z:i32=1024;const PROJ_H:i32=178;const CX:i32=160;const CY:i32=120;const SCREEN_W:i32=320;const SCREEN_H:i32=240;const CLIP_VERT_CAP:usize=12;\n";
    code += "#[derive(Copy,Clone,Debug,PartialEq,Eq)]\n";
    code += item(&source, "struct ClipVert");
    code += "\n";
    code += "const EMPTY_CLIP_VERT:ClipVert=ClipVert{x:0,y:0,z:0,u:0,v:0,r:0,g:0,b:0};\n";
    for signature in ["fn clip_distance_c<", "fn clip_intersection(", "fn clip_polygon_plane_c<"] {
        code += item(&source, signature);
        code += "\n";
    }
    code += "struct CellPlane<const P:usize>;\n";
    code += item(
        &source,
        "impl<const P: usize> AttributedClipPlane<ClipVert> for CellPlane<P>",
    );
    code += "\n";
    code += &std::fs::read_to_string(root.join("tools/fixtures/legacy_clip.rs")).expect("legacy_clip.rs");
    code += MAIN;

    let scratch = common::Scratch::new("vox-clip-test-");
    let output = common::compile_and_run(&scratch, &code, &["--edition=2021", "-O"], &[], &root);
    println!("{}", String::from_utf8_lossy(&output.stdout).trim());
}
