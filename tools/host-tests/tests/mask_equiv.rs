//! The mesher's 64-row bit sets moved from `u64` to `psx_math::Mask64`. Every expression the
//! change rewrote is checked here against the `u64` form it replaced, over boundary patterns
//! (empty, full, single bits at 0, 31, 32 and 63, runs crossing bit 32) and a deterministic
//! random sweep.

use psx_math::Mask64;

fn to_mask(v: u64) -> Mask64 {
    Mask64::from_words(v as u32, (v >> 32) as u32)
}

fn to_u64(m: Mask64) -> u64 {
    u64::from(m.low()) | u64::from(m.high()) << 32
}

fn patterns() -> Vec<u64> {
    let mut v = vec![0, !0, 1, 1 << 31, 1 << 32, 1 << 63, 0xFFFF_FFFF, 0xFFFF_FFFF_0000_0000];
    for lo in 0..64u32 {
        for n in 1..=(64 - lo) {
            let run = if n == 64 { !0u64 } else { ((1u64 << n) - 1) << lo };
            v.push(run);
        }
    }
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    for _ in 0..200_000 {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        let r = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        // Sparse and dense mixes, as the column masks are.
        v.push(r);
        v.push(r & r.rotate_left(17));
        v.push(r | r.rotate_left(29));
    }
    v
}

#[test]
fn singles_ranges_and_sets() {
    for n in 0..64usize {
        assert_eq!(to_u64(Mask64::bit(n)), 1u64 << n);
        for hi in n..=64usize {
            let want = if hi - n == 64 { !0u64 } else { ((1u64 << (hi - n)) - 1) << n };
            assert_eq!(to_u64(Mask64::range(n, hi)), want, "range {n}..{hi}");
        }
    }
    assert_eq!(to_u64(Mask64::range(0, 64)), !0u64);
}

#[test]
fn set_operations_match_the_integer_forms() {
    let pats = patterns();
    for w in pats.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (ma, mb) = (to_mask(a), to_mask(b));
        assert_eq!(to_u64(ma.or(mb)), a | b);
        assert_eq!(to_u64(ma.and(mb)), a & b);
        assert_eq!(to_u64(ma.and_not(mb)), a & !b);
        assert_eq!(ma.is_empty(), a == 0);
        // The bit-plane shifts of build_y_cols.
        assert_eq!(to_u64(ma.shr_by(1)), a >> 1);
        assert_eq!(to_u64(ma.shl_by(1)), a << 1);
        // The two words the y-column walk reads.
        for half in 0..2u32 {
            let word = if half == 0 { ma.low() } else { ma.high() };
            assert_eq!(word, (a >> (32 * half)) as u32);
        }
        // The sky top: the highest set bit above the floor layer.
        let m = a & !1;
        let top = if m == 0 { 0 } else { 63 - m.leading_zeros() };
        let mm = ma.and_not(Mask64::bit(0));
        let ntop = if mm.is_empty() { 0 } else { 63 - mm.leading_zeros() };
        assert_eq!(top, ntop);
    }
}

#[test]
fn bit_walks_visit_the_same_bits_in_the_same_order() {
    for a in patterns() {
        let mut old = Vec::new();
        let mut m = a;
        while m != 0 {
            old.push(m.trailing_zeros());
            m &= m - 1;
        }
        let mut new = Vec::new();
        let mut mm = to_mask(a);
        while !mm.is_empty() {
            let y = mm.trailing_zeros() as usize;
            new.push(y as u32);
            mm.clear(y);
        }
        assert_eq!(old, new);
        // set, clear and test agree with the integer.
        let mut built = Mask64::EMPTY;
        for b in 0..64usize {
            if a >> b & 1 != 0 {
                built.set(b);
            }
            assert_eq!(to_mask(a).test(b), a >> b & 1 != 0);
        }
        assert_eq!(to_u64(built), a);
    }
}

#[test]
fn the_greedy_span_clears_the_same_cells() {
    for a0 in 0..64usize {
        for w in 1..=(64 - a0) {
            let span = if w == 64 { !0u64 } else { ((1u64 << w) - 1) << a0 };
            let ms = Mask64::range(a0, a0 + w);
            assert_eq!(to_u64(ms), span);
            for a in patterns().into_iter().step_by(997) {
                assert_eq!(to_u64(to_mask(a).and_not(ms)), a & !span);
            }
        }
    }
}

#[test]
fn the_blast_height_mask_shifts_the_same_way() {
    let mut s = 7u32;
    for _ in 0..100_000 {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        for y0 in -63i32..64 {
            let want = if y0 >= 0 { (s as u64) << y0 } else { (s as u64) >> (-y0) };
            let m = Mask64::from_words(s, 0);
            let got = if y0 >= 0 { m.shl_by(y0 as u32) } else { m.shr_by((-y0) as u32) };
            assert_eq!(to_u64(got), want, "{s:#x} y0={y0}");
        }
    }
}
