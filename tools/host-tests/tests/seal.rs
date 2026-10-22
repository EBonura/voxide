//! The T-junction seal must close hairlines between neighbours whose shared
//! corners truncated independently, including thin slivers running at a slant.
//! The first seal (each corner pushed away from the polygon's centre) is kept
//! here as the failing baseline: it leaves cracks on exactly these shapes.

#[path = "../../../game/src/seal.rs"]
mod seal;

type P = (i32, i32);

/// Pixel centre inside the convex polygon (any winding)? Edges on the boundary
/// count, which is generous to the GPU's top-left rule and so makes the test
/// lenient by under a pixel; the slivers below are far from that margin.
fn inside(poly: &[P], px: i32, py: i32) -> bool {
    let (cx, cy) = (px as f64 + 0.5, py as f64 + 0.5);
    let mut pos = false;
    let mut neg = false;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let e = (b.0 - a.0) as f64 * (cy - a.1 as f64) - (b.1 - a.1) as f64 * (cx - a.0 as f64);
        if e > 0.0 {
            pos = true;
        }
        if e < 0.0 {
            neg = true;
        }
    }
    !(pos && neg)
}

/// The old seal: every corner two pixels away from the centre, per axis.
fn old_seal(v: &[P]) -> Vec<P> {
    let n = v.len() as i32;
    let sx: i32 = v.iter().map(|p| p.0).sum();
    let sy: i32 = v.iter().map(|p| p.1).sum();
    v.iter()
        .map(|&(x, y)| {
            (
                if x * n >= sx { x + 2 } else { x - 2 },
                if y * n >= sy { y + 2 } else { y - 2 },
            )
        })
        .collect()
}

fn new_quad_seal(v: &[P]) -> Vec<P> {
    // v is in perimeter order (0 1 3 2 of the z-order quad): [v0, v1, v3, v2].
    let x = [v[0].0, v[1].0, v[3].0, v[2].0];
    let y = [v[0].1, v[1].1, v[3].1, v[2].1];
    let (gx, gy) = seal::grow_quad(x, y);
    vec![
        (gx[0], gy[0]),
        (gx[1], gy[1]),
        (gx[3], gy[3]),
        (gx[2], gy[2]),
    ]
}

fn new_poly_seal(v: &[P]) -> Vec<P> {
    let src: Vec<P> = v.to_vec();
    let out = std::cell::RefCell::new(src.clone());
    seal::grow_polygon(v.len(), |i| src[i], |i, x, y| out.borrow_mut()[i] = (x, y));
    out.into_inner()
}

/// Tiny deterministic generator.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }
    fn unit(&mut self) -> f64 {
        self.next() as f64 / (1u64 << 31) as f64
    }
}

/// A block of `cells` slivers stacked across a slanted band: each is `len` px
/// long and `width / cells` px thick, shares its LONG edges with its
/// neighbours, and has its four corners truncated on their own with a jitter
/// of up to a pixel either way (what independent projection of a T-junction
/// does). `end_to_end` instead lays the cells along the band, sharing short
/// edges. Returns every cell as a perimeter-ordered quad plus the exact union.
fn strip(
    rng: &mut Rng,
    ox: i32,
    oy: i32,
    len: f64,
    width: f64,
    slope: f64,
    cells: usize,
    end_to_end: bool,
) -> (Vec<Vec<P>>, Vec<P>) {
    let ang = slope.atan();
    let (dx, dy) = (ang.cos(), ang.sin());
    let (nx, ny) = (-dy, dx);
    let corner = |s: f64, t: f64| -> (f64, f64) {
        (ox as f64 + s * dx + t * nx, oy as f64 + s * dy + t * ny)
    };
    let jit = |rng: &mut Rng, p: (f64, f64)| -> P {
        let j = |v: f64, r: f64| (v + r).floor() as i32;
        (
            j(p.0, (rng.unit() - 0.5) * 2.0),
            j(p.1, (rng.unit() - 0.5) * 2.0),
        )
    };
    let mut quads = Vec::new();
    for c in 0..cells {
        let f0 = c as f64 / cells as f64;
        let f1 = (c + 1) as f64 / cells as f64;
        let ((s0, s1), (t0, t1)) = if end_to_end {
            ((len * f0, len * f1), (0.0, width))
        } else {
            ((0.0, len), (width * f0, width * f1))
        };
        quads.push(vec![
            jit(rng, corner(s0, t0)),
            jit(rng, corner(s1, t0)),
            jit(rng, corner(s1, t1)),
            jit(rng, corner(s0, t1)),
        ]);
    }
    let r = |p: (f64, f64)| (p.0.round() as i32, p.1.round() as i32);
    let whole = vec![
        r(corner(0.0, 0.0)),
        r(corner(len, 0.0)),
        r(corner(len, width)),
        r(corner(0.0, width)),
    ];
    (quads, whole)
}

/// Pixels of the reference shape, at least `margin` px from its outer border,
/// that no grown quad covers.
fn holes(grow: &dyn Fn(&[P]) -> Vec<P>, quads: &[Vec<P>], whole: &[P], margin: f64) -> usize {
    let grown: Vec<Vec<P>> = quads.iter().map(|q| grow(q)).collect();
    let (mut x0, mut x1, mut y0, mut y1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
    for p in whole {
        x0 = x0.min(p.0);
        x1 = x1.max(p.0);
        y0 = y0.min(p.1);
        y1 = y1.max(p.1);
    }
    let dist = |px: f64, py: f64| -> f64 {
        // distance to the polygon boundary
        let mut d = f64::MAX;
        for i in 0..whole.len() {
            let a = whole[i];
            let b = whole[(i + 1) % whole.len()];
            let (ex, ey) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
            let l2 = ex * ex + ey * ey;
            let t = (((px - a.0 as f64) * ex + (py - a.1 as f64) * ey) / l2).clamp(0.0, 1.0);
            let (qx, qy) = (a.0 as f64 + t * ex, a.1 as f64 + t * ey);
            d = d.min(((px - qx).powi(2) + (py - qy).powi(2)).sqrt());
        }
        d
    };
    let mut n = 0;
    for py in y0..=y1 {
        for px in x0..=x1 {
            if !inside(whole, px, py) {
                continue;
            }
            if dist(px as f64 + 0.5, py as f64 + 0.5) < margin {
                continue;
            }
            if !grown.iter().any(|q| inside(q, px, py)) {
                n += 1;
            }
        }
    }
    n
}

fn survey(grow: &dyn Fn(&[P]) -> Vec<P>, end_to_end: bool) -> (usize, usize) {
    let mut rng = Rng(0x5ea1);
    let (mut total, mut shapes_with_holes) = (0, 0);
    for k in 0..400 {
        let len = 40.0 + rng.unit() * 160.0;
        let width = 4.0 + rng.unit() * 14.0;
        // Slopes from flat to steeply slanted, both ways.
        let slope = (rng.unit() - 0.5) * 2.2;
        let cells = 2 + (k % 3);
        let (quads, whole) = strip(&mut rng, 100, 100, len, width, slope, cells, end_to_end);
        let h = holes(grow, &quads, &whole, 3.0);
        total += h;
        if h > 0 {
            shapes_with_holes += 1;
        }
    }
    (total, shapes_with_holes)
}

fn survey2(grow: &dyn Fn(&[P]) -> Vec<P>, end_to_end: bool, seed: u64, n: usize) -> (usize, usize) {
    let mut rng = Rng(seed);
    let (mut total, mut shapes) = (0, 0);
    for k in 0..n {
        let len = 20.0 + rng.unit() * 230.0;
        let width = 2.0 + rng.unit() * 16.0;
        let slope = (rng.unit() - 0.5) * 6.0; // up to about +-71 degrees
        let cells = 2 + (k % 4);
        let (quads, whole) = strip(&mut rng, 150, 150, len, width, slope, cells, end_to_end);
        let h = holes(grow, &quads, &whole, 3.0);
        total += h;
        if h > 0 {
            shapes += 1;
        }
    }
    (total, shapes)
}

/// The rule written out directly, with the winding taken from the cross product:
/// agree components move `ka`, conflicting ones `kc` in the sign of the longer edge.
fn rule_quad(v: &[P], ka: i32, kc: i32) -> Vec<P> {
    let (x0, y0, x1, y1, x3, y3, x2, y2) = (
        v[0].0, v[0].1, v[1].0, v[1].1, v[2].0, v[2].1, v[3].0, v[3].1,
    );
    let ax = (x1 - x0) + (x3 - x2);
    let ay = (y1 - y0) + (y3 - y2);
    let bx = (x2 - x0) + (x3 - x1);
    let by = (y2 - y0) + (y3 - y1);
    let det = ax * by - ay * bx;
    let sd = if det >= 0 { 1 } else { -1 };
    let sg0 = |v: i32| (v > 0) as i32 - (v < 0) as i32;
    // A component under a quarter of the other counts as zero.
    let dz = |x: i32, y: i32| {
        (
            if x.abs() * 4 < y.abs() { 0 } else { sg0(x) },
            if y.abs() * 4 < x.abs() { 0 } else { sg0(y) },
        )
    };
    let (sax, say) = dz(ax, ay);
    let (sbx, sby) = dz(bx, by);
    let u = (-sd * say, sd * sax);
    let w = (sd * sby, -sd * sbx);
    let a_long = (ax.abs() + ay.abs()) >= (bx.abs() + by.abs());
    let comb = |sp: i32, sq: i32, p_is_a_edge: bool| -> i32 {
        if sp == sq {
            sp * ka
        } else if sp == 0 {
            sq * ka
        } else if sq == 0 {
            sp * ka
        } else {
            let p_longer = p_is_a_edge == a_long;
            (if p_longer { sp } else { sq }) * kc
        }
    };
    let neg = |p: (i32, i32)| (-p.0, -p.1);
    let d = |p: (i32, i32), q: (i32, i32)| (comb(p.0, q.0, true), comb(p.1, q.1, true));
    let d0 = d(neg(u), neg(w));
    let d1 = d(neg(u), w);
    let d2 = d(u, neg(w));
    let d3 = d(u, w);
    vec![
        (x0 + d0.0, y0 + d0.1),
        (x1 + d1.0, y1 + d1.1),
        (x3 + d3.0, y3 + d3.1),
        (x2 + d2.0, y2 + d2.1),
    ]
}

/// Perimeter-ordered wrapper of the written-out rule.
fn reference_quad(v: &[P], ka: i32, kc: i32) -> Vec<P> {
    rule_quad(v, ka, kc)
}

#[test]
fn the_old_corner_push_leaves_cracks_between_slanted_slivers() {
    let (total, shapes) = survey(&old_seal, false);
    eprintln!("old seal, side by side: {total} holes in {shapes} of 400 bands");
    assert!(
        total > 1000 && shapes > 100,
        "baseline should crack: {total} holes in {shapes} bands"
    );
    // Cells laid along their own length overlap by sliding, which is why the
    // flaw hid for so long.
    let (total, _) = survey(&old_seal, true);
    eprintln!("old seal, end to end: {total} holes");
}

#[test]
fn grown_quads_leave_no_crack_between_slanted_slivers() {
    for end_to_end in [false, true] {
        let (total, shapes) = survey(&new_quad_seal, end_to_end);
        assert_eq!(
            (total, shapes),
            (0, 0),
            "grow_quad left holes (end_to_end {end_to_end})"
        );
        let (total, shapes) = survey2(&new_quad_seal, end_to_end, 77, 600);
        assert_eq!(
            (total, shapes),
            (0, 0),
            "grow_quad left holes in the wide survey (end_to_end {end_to_end})"
        );
    }
}

#[test]
fn grown_polygons_leave_no_crack_between_slanted_slivers() {
    for end_to_end in [false, true] {
        let (total, shapes) = survey(&new_poly_seal, end_to_end);
        assert_eq!(
            (total, shapes),
            (0, 0),
            "grow_polygon left holes (end_to_end {end_to_end})"
        );
        let (total, shapes) = survey2(&new_poly_seal, end_to_end, 77, 600);
        assert_eq!(
            (total, shapes),
            (0, 0),
            "grow_polygon left holes in the wide survey (end_to_end {end_to_end})"
        );
    }
}

#[test]
fn the_table_matches_the_rule_written_out() {
    let mut rng = Rng(0xa11ce);
    for _ in 0..20000 {
        let mut p = |lo: i32, hi: i32| lo + (rng.next() % (hi - lo + 1) as u32) as i32;
        // A random convex-ish quad, possibly degenerate.
        let (ox, oy) = (p(-50, 50), p(-50, 50));
        let (ax, ay, bx, by) = (p(-40, 40), p(-40, 40), p(-40, 40), p(-40, 40));
        let (sx, sy) = (p(-6, 6), p(-6, 6)); // far-corner skew
        let q = [
            (ox, oy),
            (ox + ax, oy + ay),
            (ox + ax + bx + sx, oy + ay + by + sy),
            (ox + bx, oy + by),
        ];
        let got = new_quad_seal(&q);
        let want = reference_quad(&q, seal::GROW, seal::GROW_TIP);
        assert_eq!(got, want, "quad {q:?}");
    }
}

#[test]
fn a_square_grows_by_the_seal_on_every_side() {
    let sq = [(100, 100), (110, 100), (110, 110), (100, 110)];
    let g = seal::GROW;
    let want = vec![
        (100 - g, 100 - g),
        (110 + g, 100 - g),
        (110 + g, 110 + g),
        (100 - g, 110 + g),
    ];
    assert_eq!(new_quad_seal(&sq), want);
    assert_eq!(new_poly_seal(&sq), want);
    // And the same square wound the other way.
    let rev = [(100, 100), (100, 110), (110, 110), (110, 100)];
    let mut got = new_poly_seal(&rev);
    got.sort();
    let mut w = want.clone();
    w.sort();
    assert_eq!(got, w);
}

#[test]
fn no_corner_moves_further_than_the_grow() {
    let mut rng = Rng(9);
    for _ in 0..20000 {
        let mut p = |lo: i32, hi: i32| lo + (rng.next() % (hi - lo + 1) as u32) as i32;
        let q = [
            (p(-300, 300), p(-300, 300)),
            (p(-300, 300), p(-300, 300)),
            (p(-300, 300), p(-300, 300)),
            (p(-300, 300), p(-300, 300)),
        ];
        for (a, b) in q.iter().zip(new_quad_seal(&q).iter()) {
            assert!(
                (a.0 - b.0).abs() <= seal::GROW && (a.1 - b.1).abs() <= seal::GROW,
                "{a:?} -> {b:?}"
            );
        }
    }
}

#[test]
fn a_degenerate_polygon_is_left_alone() {
    let line = [(100, 100), (200, 100), (300, 100)];
    assert_eq!(new_poly_seal(&line), line.to_vec());
}

#[test]
fn polygons_past_the_x_rail_stay_inside_it() {
    let sq = [(1015, 50), (1023, 50), (1023, 60), (1015, 60)];
    for (_, p) in new_poly_seal(&sq).iter().enumerate() {
        assert!(p.0 <= 1023 && p.0 >= -1024, "{p:?}");
    }
}

/// Two adjacent near-band cells measured on the recorded walk (lockstep poll 991,
/// before the seal): they share the edge (0,200)-(66,170) exactly. The second
/// polygon's corners come in the order the clipper left them, and its first three
/// are nearly collinear (cross +6) although its area is negative: reading the
/// winding from those three grew the polygon inward and opened a 60 px crack.
#[test]
fn adjacent_clipped_cells_overlap_even_when_the_first_corners_are_collinear() {
    let p1 = [(0, 167), (36, 156), (66, 170), (0, 200)];
    let p2 = [
        (0, 208),
        (33, 238),
        (35, 240),
        (109, 190),
        (66, 170),
        (0, 200),
    ];
    let g1 = new_poly_seal(&p1);
    let g2 = new_poly_seal(&p2);
    // Every pixel within a pixel or two of the shared edge, on either side, is
    // covered by one of the two grown polygons.
    let mut holes = 0;
    for px in 4..62 {
        // the edge's y at this x, then the pixels just above and below it
        let y = 200.0 - 30.0 * px as f64 / 66.0;
        for dy in -3..=3 {
            let py = (y + dy as f64).floor() as i32;
            if !inside(&g1, px, py) && !inside(&g2, px, py) {
                holes += 1;
            }
        }
    }
    assert_eq!(holes, 0, "grown {g1:?} and {g2:?}");
    // The shared edge's corners moved toward the other polygon, not into their own.
    let edge_corner =
        |g: &Vec<P>, orig: &[P], at: P| -> P { g[orig.iter().position(|&p| p == at).unwrap()] };
    let (a1, a2) = (
        edge_corner(&g1, &p1, (66, 170)),
        edge_corner(&g2, &p2, (66, 170)),
    );
    assert!(a1.0 > 66 && a1.1 >= 170, "{a1:?}");
    assert!(a2.0 <= 66 && a2.1 <= 170, "{a2:?}");
}
