struct C {
    z: i32,
}
fn actual(depths: &[i32]) -> usize {
    let a: Vec<C> = depths.iter().map(|&z| C { z }).collect();
    let n = a.len();
    // The farthest depth from any candidate is one of the two extrema.
    // Keep the first minimum exactly as the former all-pairs search did.
    let mut min_z = a[0].z;
    let mut max_z = min_z;
    let mut j = 1usize;
    while j < n {
        min_z = min_z.min(a[j].z);
        max_z = max_z.max(a[j].z);
        j += 1;
    }
    let mut pivot = 0usize;
    let mut best = i32::MAX;
    let mut candidate = 0usize;
    while candidate < n {
        let worst = (a[candidate].z - min_z).max(max_z - a[candidate].z);
        if worst < best {
            best = worst;
            pivot = candidate;
        }
        candidate += 1;
    }

    pivot
}
fn reference(d: &[i32]) -> usize {
    (0..d.len())
        .min_by_key(|&i| d.iter().map(|&z| (d[i] - z).abs()).max().unwrap())
        .unwrap()
}
#[test]
fn exhaustive_order_and_ties() {
    let values = [18, 19, 50, 128, 1024];
    for n in 1..=8 {
        for code in 0..5usize.pow(n) {
            let mut v = code;
            let mut d = vec![0; n as usize];
            for z in &mut d {
                *z = values[v % 5];
                v /= 5;
            }
            assert_eq!(actual(&d), reference(&d), "{:?}", d);
        }
    }
}
#[test]
fn twelve_corners_and_extreme_positive_depths() {
    let mut seed = 0x773329abu32;
    for i in 0..100000usize {
        let n = 1 + i % 12;
        let mut d = vec![0; n];
        for z in &mut d {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *z = (seed % 65535 + 1) as i32;
        }
        assert_eq!(actual(&d), reference(&d), "{:?}", d);
    }
    for d in [
        [1, 65535, 32768, 32767],
        [65535, 1, 32767, 32768],
        [18, 18, 18, 18],
    ] {
        assert_eq!(actual(&d), reference(&d));
    }
}
