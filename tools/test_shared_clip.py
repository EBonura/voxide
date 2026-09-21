"""Check actual production clipping adapter against the frozen renderer oracle."""
from pathlib import Path
import subprocess, tempfile, unittest
ROOT=Path(__file__).resolve().parents[1]
def item(text, signature):
    start=text.index(signature); i=text.index('{',start); depth=1; j=i+1
    while depth:
        depth+=(text[j]=='{')-(text[j]=='}'); j+=1
    return text[start:j]
class ClipEquivalence(unittest.TestCase):
    def test_actual_adapter_preserves_vertices_attributes_and_order(self):
        source=(ROOT/'game/src/main.rs').read_text()
        kernel=ROOT/'.psoxide/sdk/crates/psx-math/src'
        code='#![allow(dead_code)]\n'
        code+=f'#[path="{kernel / "int32.rs"}"] mod int32;\n'
        code+=f'#[path="{kernel / "attributed_clip.rs"}"] mod attributed_clip;\n'
        code+='use attributed_clip::{AttributedClipPlane,ClipTraversal,clip_convex_plane};\n'
        code+='const NEAR_Z:i32=18;const FAR_Z:i32=1024;const PROJ_H:i32=178;const CX:i32=160;const CY:i32=120;const SCREEN_W:i32=320;const SCREEN_H:i32=240;const CLIP_VERT_CAP:usize=12;\n'
        code+='#[derive(Copy,Clone,Debug,PartialEq,Eq)]\n'+item(source,'struct ClipVert')+'\n'
        code+='const EMPTY_CLIP_VERT:ClipVert=ClipVert{x:0,y:0,z:0,u:0,v:0,r:0,g:0,b:0};\n'
        for signature in ['fn clip_distance_c<','fn clip_intersection(','fn clip_polygon_plane_c<']:
            code+=item(source,signature)+'\n'
        code+='struct CellPlane<const P:usize>;\n'+item(source,'impl<const P: usize> AttributedClipPlane<ClipVert> for CellPlane<P>')+'\n'
        code+=(ROOT/'tools/fixtures/legacy_clip.rs').read_text()
        code+=r"""
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
"""
        with tempfile.TemporaryDirectory(prefix='vox-clip-test-') as tmp:
            path=Path(tmp);(path/'main.rs').write_text(code)
            subprocess.run(['rustc','--edition=2021','-O',str(path/'main.rs'),'-o',str(path/'test')],check=True)
            result=subprocess.run([str(path/'test')],check=True,text=True,capture_output=True)
            print(result.stdout.strip())
if __name__=='__main__':unittest.main()
