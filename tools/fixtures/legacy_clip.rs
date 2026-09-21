// Frozen VoXide 76b071e clip traversal; regression oracle only.
fn legacy_clip<const P: usize>(
    src: &[ClipVert; CLIP_VERT_CAP],
    src_n: usize,
    dst: &mut [ClipVert; CLIP_VERT_CAP],
) -> usize {
    let mut out = 0usize;
    let mut previous = src[src_n - 1];
    let mut previous_d = clip_distance_c::<P>(&previous);
    let mut i = 0usize;
    while i < src_n {
        let current = src[i];
        let current_d = clip_distance_c::<P>(&current);
        let previous_in = previous_d >= 0;
        let current_in = current_d >= 0;
        if previous_in != current_in && out < CLIP_VERT_CAP {
            dst[out] = clip_intersection(previous, current, previous_d, current_d);
            out += 1;
        }
        if current_in && out < CLIP_VERT_CAP {
            dst[out] = current;
            out += 1;
        }
        previous = current;
        previous_d = current_d;
        i += 1;
    }
    out
}
