//! Ear-clipping triangulation of planar polygons, so concave faces (an L-shaped
//! room drawn with the Line tool) render, pick and export correctly.

/// Triangulates one planar face ring (vertex indices into `positions`) by ear
/// clipping. Triangles keep the ring's winding. Degenerate input falls back to
/// a fan so that every face still produces `ring.len() - 2` triangles.
pub fn triangulate(positions: &[[f64; 3]], ring: &[u32]) -> Vec<[u32; 3]> {
    let n = ring.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[ring[0], ring[1], ring[2]]];
    }

    // Project onto the plane most perpendicular to the Newell normal.
    let normal = newell(positions, ring);
    let (ax, ay) = match dominant_axis(normal) {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let pts: Vec<[f64; 2]> = ring
        .iter()
        .map(|&v| {
            let p = positions[v as usize];
            [p[ax], p[ay]]
        })
        .collect();
    // After dropping the dominant axis the ring may appear clockwise; ears are
    // convex corners relative to the ring's own orientation.
    let orientation = signed_area_2d(&pts).signum();

    let (mut out, idx) = ears(ring, &pts, orientation, Ring::Outline);
    // Three left, or no ear found (collinear or self-touching input): fan the rest.
    for i in 1..idx.len() - 1 {
        out.push([ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]]);
    }
    out
}

/// Triangulates a face with holes, the holes left open. Each hole is joined to the outline by a bridge (a cut walked there
/// and back), the hole reaching furthest along the face first, so the face becomes one outline for ear clipping.
pub fn triangulate_with_holes(positions: &[[f64; 3]], outer: &[u32], holes: &[Vec<u32>]) -> Vec<[u32; 3]> {
    if outer.len() < 3 {
        return Vec::new();
    }
    let (ring, pts) = bridged(positions, outer, holes);
    clip(&ring, &pts, 1.0)
}

/// The outline of a face with holes with every hole joined to it by a bridge (see [`triangulate_with_holes`]), and its
/// points flattened (counter-clockwise).
fn bridged(positions: &[[f64; 3]], outer: &[u32], holes: &[Vec<u32>]) -> (Vec<u32>, Vec<[f64; 2]>) {
    let (ax, ay) = match dominant_axis(newell(positions, outer)) {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let flat = |v: u32| {
        let p = positions[v as usize];
        [p[ax], p[ay]]
    };
    let area = |r: &[u32]| signed_area_2d(&r.iter().map(|&v| flat(v)).collect::<Vec<_>>());
    // worked with the outline counter-clockwise (x mirrored if it is not) and every hole clockwise
    let mirror = if area(outer) < 0.0 { -1.0 } else { 1.0 };
    let pt = |v: u32| {
        let q = flat(v);
        [q[0] * mirror, q[1]]
    };
    let mut ring: Vec<u32> = outer.to_vec();
    let mut hs: Vec<Vec<u32>> = holes
        .iter()
        .filter(|h| h.len() >= 3)
        .map(|h| {
            let mut h = h.clone();
            if area(&h) * mirror > 0.0 {
                h.reverse();
            }
            h
        })
        .collect();
    let reach = |h: &Vec<u32>| h.iter().map(|&v| pt(v)[0]).fold(f64::MIN, f64::max);
    hs.sort_by(|a, b| reach(b).total_cmp(&reach(a)));
    for h in hs {
        // M: the hole's corner furthest along x
        let mi = (0..h.len()).max_by(|&a, &b| pt(h[a])[0].total_cmp(&pt(h[b])[0])).expect("three corners");
        let m = pt(h[mi]);
        // the nearest edge of the outline (holes joined so far included) met by a ray from M along +x
        let mut hit: Option<(f64, usize)> = None;
        for i in 0..ring.len() {
            let (a, b) = (pt(ring[i]), pt(ring[(i + 1) % ring.len()]));
            if (a[1] - m[1]) * (b[1] - m[1]) > 0.0 || a[1] == b[1] {
                continue;
            }
            let x = a[0] + (m[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if x >= m[0] && hit.is_none_or(|(bx, _)| x < bx) {
                hit = Some((x, i));
            }
        }
        let pi = match hit {
            Some((x, i)) => {
                let j = (i + 1) % ring.len();
                // the end of that edge further along x, unless another corner inside the triangle M, I, P would block the
                // bridge: then the one of those at the smallest angle from the ray
                let p = if pt(ring[i])[0] >= pt(ring[j])[0] { i } else { j };
                let (pp, ip) = (pt(ring[p]), [x, m[1]]);
                let blocking = (0..ring.len())
                    .filter(|&k| k != p && ring[k] != ring[p] && pt(ring[k])[0] > m[0] && in_triangle(pt(ring[k]), m, ip, pp))
                    .min_by(|&a, &b| {
                        let ang = |k: usize| {
                            let q = pt(ring[k]);
                            ((q[1] - m[1]).abs() / (q[0] - m[0]), (q[0] - m[0]).powi(2) + (q[1] - m[1]).powi(2))
                        };
                        ang(a).partial_cmp(&ang(b)).unwrap_or(std::cmp::Ordering::Equal)
                    });
                blocking.unwrap_or(p)
            }
            // nothing to the right (a hole touching the outline): the nearest corner
            None => (0..ring.len())
                .min_by(|&a, &b| {
                    let d = |k: usize| (pt(ring[k])[0] - m[0]).powi(2) + (pt(ring[k])[1] - m[1]).powi(2);
                    d(a).total_cmp(&d(b))
                })
                .expect("an outline"),
        };
        // the corner may be on the outline twice (a bridge to an earlier hole left from it): the copy whose corner opens
        // toward M, or the new bridge would cross the old one
        let pi = {
            let v = ring[pi];
            let copies: Vec<usize> = (0..ring.len()).filter(|&k| ring[k] == v).collect();
            let len = ring.len();
            copies.iter().copied().find(|&k| in_cone(pt(ring[(k + len - 1) % len]), pt(ring[k]), pt(ring[(k + 1) % len]), m)).unwrap_or(pi)
        };
        let mut joined = Vec::with_capacity(ring.len() + h.len() + 2);
        joined.extend_from_slice(&ring[..=pi]);
        joined.extend(h[mi..].iter().chain(&h[..mi]).copied());
        joined.push(h[mi]);
        joined.extend_from_slice(&ring[pi..]);
        ring = joined;
    }
    let pts: Vec<[f64; 2]> = ring.iter().map(|&v| pt(v)).collect();
    (ring, pts)
}

/// Whether point `b` lies in the open corner at `a` between its neighbours `prev` and `next`, the outline running
/// counter-clockwise (O'Rourke's InCone).
fn in_cone(prev: [f64; 2], a: [f64; 2], next: [f64; 2], b: [f64; 2]) -> bool {
    let left = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| cross_2d(p, q, r) > 0.0;
    let left_on = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| cross_2d(p, q, r) >= 0.0;
    if left_on(a, next, prev) {
        left(a, b, prev) && left(b, a, next)
    } else {
        !(left_on(a, b, next) && left_on(b, a, prev))
    }
}

/// Ear clipping of `ring` (vertex numbers, a number may come twice along a bridge) with its points `pts`, wound as
/// `orientation` says. A point with the same number as a corner of the ear does not block it.
fn clip(ring: &[u32], pts: &[[f64; 2]], orientation: f64) -> Vec<[u32; 3]> {
    let (mut out, idx) = ears(ring, pts, orientation, Ring::Bridged);
    for i in 1..idx.len().saturating_sub(1) {
        let t = [ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]];
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            out.push(t);
        }
    }
    out
}

/// What is being clipped: a face's own outline ([`triangulate`]), or an outline with its holes bridged in ([`clip`]). Along a
/// bridge a corner comes twice, so there a corner with the same vertex number as one of the ear's does not block it (in an
/// outline only the ear's own three places do not); and when no ear is left, a corner on a straight line is dropped and
/// clipping goes on (an outline stops there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ring {
    Outline,
    Bridged,
}

impl Ring {
    /// Whether the corner at place `p` counts as one of the ear `a`, `b`, `c`'s own corners (it does not block the ear).
    fn corner(self, ring: &[u32], p: usize, a: usize, b: usize, c: usize) -> bool {
        match self {
            Ring::Outline => p == a || p == b || p == c,
            Ring::Bridged => ring[p] == ring[a] || ring[p] == ring[b] || ring[p] == ring[c],
        }
    }
}

/// Rings with at least this many corners are clipped by [`ears_fast`], smaller ones by [`ears_plain`] (quicker for them).
/// Both cut the same triangles.
const FAST_FROM: usize = 160;

/// Ear clipping: each time the first ear along the ring is cut off, until three corners are left or no ear is found.
/// Returns the triangles and the places (in `ring`) of the corners left, in order, for the caller to fan.
fn ears(ring: &[u32], pts: &[[f64; 2]], orientation: f64, kind: Ring) -> (Vec<[u32; 3]>, Vec<usize>) {
    if ring.len() >= FAST_FROM {
        ears_fast(ring, pts, orientation, kind)
    } else {
        ears_plain(ring, pts, orientation, kind)
    }
}

/// [`ears`] the plain way: the ring is walked from its start for every triangle, each corner on the way tested against all
/// the corners left (as many tests as corners squared, or more).
fn ears_plain(ring: &[u32], pts: &[[f64; 2]], orientation: f64, kind: Ring) -> (Vec<[u32; 3]>, Vec<usize>) {
    let n = ring.len();
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let turn = cross_2d(pts[a], pts[b], pts[c]) * orientation;
            if turn <= 0.0 {
                return false;
            }
            !idx.iter().any(|&p| !kind.corner(ring, p, a, b, c) && in_triangle(pts[p], pts[a], pts[b], pts[c]))
        });
        match ear {
            Some(i) => {
                let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                out.push([ring[a], ring[b], ring[c]]);
                idx.remove(i);
            }
            // a bridge's two sides meet with no area between them: a corner on a straight line goes without a triangle
            None if kind == Ring::Bridged => {
                match (0..m).find(|&i| cross_2d(pts[idx[(i + m - 1) % m]], pts[idx[i]], pts[idx[(i + 1) % m]]).abs() < 1e-18) {
                    Some(i) => {
                        idx.remove(i);
                    }
                    None => break,
                }
            }
            None => break,
        }
    }
    (out, idx)
}

/// What is known about a corner being an ear (see [`ears_fast`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ear {
    /// To be tested: new, or its neighbours or the corner that blocked it have gone since.
    Unknown,
    /// It does not turn the ring's way.
    Reflex,
    /// The corner at this place lies in its triangle.
    Blocked(u32),
    Yes,
}

/// [`ears`] for big rings, cutting exactly the triangles [`ears_plain`] cuts, in the same order. Every corner's answer (an
/// ear or not, and what blocks it) is kept until it can change: when a neighbour is cut off (its triangle is another one
/// then) or the corner blocking it goes (corners only ever go, which cannot make an ear stop being one). The first ear
/// along the ring is then the first of the ears known, unless a corner before it still to be tested turns out to be one.
/// And a triangle is tested only against the corners near it (see [`Grid::near`]), which answers as testing them all does.
fn ears_fast(ring: &[u32], pts: &[[f64; 2]], orientation: f64, kind: Ring) -> (Vec<[u32; 3]>, Vec<usize>) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = ring.len();
    let grid = Grid::new(pts);
    // the corners left, as a ring in their order: neighbours either way, the first one, how many
    let mut next: Vec<usize> = (0..n).map(|i| (i + 1) % n).collect();
    let mut prev: Vec<usize> = (0..n).map(|i| (i + n - 1) % n).collect();
    let mut alive = vec![true; n];
    let (mut head, mut left) = (0, n);
    let mut state = vec![Ear::Unknown; n];
    // the corners each corner blocks: a list per corner threaded through one arena (an entry is out of date once that corner
    // has been tested again)
    let mut blocking = vec![u32::MAX; n];
    let mut arena: Vec<(u32, u32)> = Vec::new();
    // corners to test and ears known, the first along the ring on top (entries out of date are skipped)
    let mut unknown: BinaryHeap<Reverse<usize>> = (0..n).map(Reverse).collect();
    let mut known: BinaryHeap<Reverse<usize>> = BinaryHeap::new();
    let mut out = Vec::with_capacity(n.saturating_sub(2));
    while left > 3 {
        let first = loop {
            while unknown.peek().is_some_and(|&Reverse(v)| !alive[v] || state[v] != Ear::Unknown) {
                unknown.pop();
            }
            while known.peek().is_some_and(|&Reverse(v)| !alive[v] || state[v] != Ear::Yes) {
                known.pop();
            }
            let ear = known.peek().map(|&Reverse(v)| v);
            let Some(&Reverse(b)) = unknown.peek().filter(|&&Reverse(v)| ear.is_none_or(|e| v < e)) else { break ear };
            unknown.pop();
            // the test ears_plain makes
            let (a, c) = (prev[b], next[b]);
            let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
            let area2 = cross_2d(pa, pb, pc);
            let turn = area2 * orientation;
            state[b] = if turn <= 0.0 {
                Ear::Reflex
            } else {
                let blocks = |p: usize| alive[p] && !kind.corner(ring, p, a, b, c) && in_triangle(pts[p], pa, pb, pc);
                let found = match grid.as_ref().and_then(|g| g.near(pa, pb, pc, area2)) {
                    Some(mut near) => near.find(|&p| blocks(p)),
                    None => {
                        let mut v = head;
                        (0..left).find_map(|_| {
                            let p = v;
                            v = next[v];
                            blocks(p).then_some(p)
                        })
                    }
                };
                found.map_or(Ear::Yes, |q| Ear::Blocked(q as u32))
            };
            match state[b] {
                Ear::Yes => known.push(Reverse(b)),
                Ear::Blocked(q) => {
                    arena.push((b as u32, blocking[q as usize]));
                    blocking[q as usize] = (arena.len() - 1) as u32;
                }
                _ => {}
            }
        };
        let gone = match first {
            Some(b) => {
                out.push([ring[prev[b]], ring[b], ring[next[b]]]);
                b
            }
            // as in ears_plain: the first corner along the ring on a straight line goes without a triangle
            None if kind == Ring::Bridged => {
                let mut v = head;
                let flat = (0..left).find_map(|_| {
                    let p = v;
                    v = next[v];
                    (cross_2d(pts[prev[p]], pts[p], pts[next[p]]).abs() < 1e-18).then_some(p)
                });
                match flat {
                    Some(p) => p,
                    None => break,
                }
            }
            None => break,
        };
        let (a, c) = (prev[gone], next[gone]);
        next[a] = c;
        prev[c] = a;
        alive[gone] = false;
        left -= 1;
        if head == gone {
            head = c;
        }
        // its neighbours' triangles change; the corners it blocked may be ears now
        for v in [a, c] {
            state[v] = Ear::Unknown;
            unknown.push(Reverse(v));
        }
        let mut k = blocking[gone];
        while k != u32::MAX {
            let (v, after) = arena[k as usize];
            let v = v as usize;
            if alive[v] && state[v] == Ear::Blocked(gone as u32) {
                state[v] = Ear::Unknown;
                unknown.push(Reverse(v));
            }
            k = after;
        }
    }
    let mut rest = Vec::with_capacity(left);
    let mut v = head;
    for _ in 0..left {
        rest.push(v);
        v = next[v];
    }
    (out, rest)
}

/// Points further than this from the origin keep a ring out of the [`Grid`] (no product of two coordinates may overflow).
const GRID_REACH: f64 = 1e100;
/// A triangle smaller than this across is tested against every corner (underflow is not worth bounding).
const GRID_SMALLEST: f64 = 1e-100;
/// A triangle thinner than this (twice its area over the square of its size) is tested against every corner: see
/// [`Grid::near`]. 2^-29, which is 2^24 times the precision of a double.
const GRID_SLIVER: f64 = 1.0 / 536_870_912.0;

/// A ring's points sorted into square cells, so the corners near a triangle are found without looking at all of them.
struct Grid {
    lo: [f64; 2],
    /// Cells per unit of length, and the size of a cell.
    inv: f64,
    cell: f64,
    /// More than rounding can put a point outside its cell's square (see `col`) or misplace that square's corners.
    loose: f64,
    cols: usize,
    rows: usize,
    /// Where each cell's points (row by row) start in `items`, and one more at the end.
    start: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    /// About one cell per point. None when a point is not a finite number within [`GRID_REACH`] or the points are all
    /// within [`GRID_SMALLEST`] of each other (every corner is then tested).
    fn new(pts: &[[f64; 2]]) -> Option<Grid> {
        let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for p in pts {
            for k in 0..2 {
                if p[k].is_nan() || p[k].abs() > GRID_REACH {
                    return None;
                }
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let size = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if size.is_nan() || size < GRID_SMALLEST {
            return None;
        }
        let side = (pts.len() as f64).sqrt().ceil();
        let inv = side / size;
        let along = |k: usize| (((hi[k] - lo[k]) * inv) as usize).min(side as usize) + 1;
        let cell = 1.0 / inv;
        let reach = lo[0].abs().max(lo[1].abs()).max(hi[0].abs()).max(hi[1].abs());
        let loose = 1e-9 * (cell + reach);
        let mut g = Grid { lo, inv, cell, loose, cols: along(0), rows: along(1), start: Vec::new(), items: Vec::new() };
        let cell: Vec<usize> = pts.iter().map(|p| g.row(p[1]) * g.cols + g.col(p[0])).collect();
        let mut start = vec![0u32; g.cols * g.rows + 1];
        for &c in &cell {
            start[c + 1] += 1;
        }
        for i in 1..start.len() {
            start[i] += start[i - 1];
        }
        let mut fill = start.clone();
        let mut items = vec![0u32; pts.len()];
        for (i, &c) in cell.iter().enumerate() {
            items[fill[c] as usize] = i as u32;
            fill[c] += 1;
        }
        g.start = start;
        g.items = items;
        Some(g)
    }

    /// The column of x: never smaller for a larger x, so rounding cannot put a point on the wrong side of a query's edge.
    fn col(&self, x: f64) -> usize {
        (((x - self.lo[0]) * self.inv) as isize).clamp(0, self.cols as isize - 1) as usize
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.lo[1]) * self.inv) as isize).clamp(0, self.rows as isize - 1) as usize
    }

    /// The points (places in the ring) in the cells round triangle `a`, `b`, `c` (`area2`: twice its area as `cross_2d`
    /// works it out): every point for which `in_triangle` can come out true, and some more. None for a sliver or a speck,
    /// which is then tested against every point.
    ///
    /// Why no other point can come out inside: write `w` for the triangle's size (the longer side of its box), `T` for twice
    /// its true area (at least `2^23 u w^2` when the rounded one passes the test here; u = 2^-53, the precision of a double),
    /// and take a point at distance `d` outside the box. Each of `in_triangle`'s three cross products comes out within
    /// `E <= 4u (|dx1 dy2| + |dy1 dx2|) <= 8u w (d + w)` of its true value (coordinates within `GRID_REACH`: no overflow;
    /// underflow is negligible above `GRID_SMALLEST`), and the three true values add up to `T`. For none of the three to come
    /// out on the other side, either all are above `-E` (the point's barycentric coordinates are then all above `-E/T`,
    /// which keeps it within `9 E w / T <= 1e-5 (d + w)` of the triangle, so `d` is at most about `1e-5 w` and the point is
    /// that close to the triangle itself), or all are below `E` (which needs `T <= 3E`: impossible within `2w` of the box,
    /// and further out the largest of the three grows faster than `E` does). So the cells looked at are those within a
    /// thousandth of `w` of the triangle: row by row, the columns the triangle spans in that row (both grown by that), and
    /// one cell more each way; not the whole of its box, which a long thin triangle mostly does not cover.
    fn near(&self, a: [f64; 2], b: [f64; 2], c: [f64; 2], area2: f64) -> Option<impl Iterator<Item = usize> + '_> {
        let (x0, x1) = (a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0]));
        let (y0, y1) = (a[1].min(b[1]).min(c[1]), a[1].max(b[1]).max(c[1]));
        let w = (x1 - x0).max(y1 - y0);
        if !(w >= GRID_SMALLEST && area2.abs() >= GRID_SLIVER * w * w) {
            return None;
        }
        // a thousandth of its size, and more than the rounding of the coordinates themselves
        let r = w * 1e-3 + x0.abs().max(x1.abs()).max(y0.abs()).max(y1.abs()) * 1e-12;
        let (c0, c1) = (self.col(x0 - r).saturating_sub(1), (self.col(x1 + r) + 1).min(self.cols - 1));
        let (r0, r1) = (self.row(y0 - r).saturating_sub(1), (self.row(y1 + r) + 1).min(self.rows - 1));
        let grow = r + self.loose;
        Some((r0..=r1).flat_map(move |row| {
            let (lo, hi) = self.across(row, grow, [(a, b), (b, c), (c, a)]);
            let (from, to) =
                if lo <= hi { (self.col(lo - grow).saturating_sub(1).max(c0), (self.col(hi + grow) + 1).min(c1)) } else { (1, 0) };
            let k = row * self.cols;
            let (s, e) = if from <= to { (self.start[k + from] as usize, self.start[k + to + 1] as usize) } else { (0, 0) };
            self.items[s..e].iter().map(|&i| i as usize)
        }))
    }

    /// How far the triangle with these `sides` reaches left and right within row `row`'s band of the grid grown by `grow`
    /// (upside down, lo > hi, when it does not reach the band). Its widest points within a band lie on its sides, where they
    /// cross the band's edges or end inside it; rounding moves them by far less than `grow`.
    fn across(&self, row: usize, grow: f64, sides: [([f64; 2], [f64; 2]); 3]) -> (f64, f64) {
        let (ylo, yhi) = (self.lo[1] + row as f64 * self.cell - grow, self.lo[1] + (row + 1) as f64 * self.cell + grow);
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for (s, t) in sides {
            let (from, to) = (s[1].min(t[1]).max(ylo), s[1].max(t[1]).min(yhi));
            if from > to {
                continue;
            }
            let (left, right) = (s[0].min(t[0]), s[0].max(t[0]));
            if s[1] == t[1] {
                (lo, hi) = (lo.min(left), hi.max(right));
                continue;
            }
            for y in [from, to] {
                let x = (s[0] + (y - s[1]) * (t[0] - s[0]) / (t[1] - s[1])).clamp(left, right);
                (lo, hi) = (lo.min(x), hi.max(x));
            }
        }
        (lo, hi)
    }
}

fn newell(positions: &[[f64; 3]], ring: &[u32]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..ring.len() {
        let a = positions[ring[i] as usize];
        let b = positions[ring[(i + 1) % ring.len()] as usize];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

fn dominant_axis(n: [f64; 3]) -> usize {
    let a = n.map(f64::abs);
    if a[0] >= a[1] && a[0] >= a[2] {
        0
    } else if a[1] >= a[2] {
        1
    } else {
        2
    }
}

fn signed_area_2d(pts: &[[f64; 2]]) -> f64 {
    let mut s = 0.0;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    s * 0.5
}

fn cross_2d(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Inclusive point-in-triangle test, independent of the triangle's winding.
fn in_triangle(p: [f64; 2], a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> bool {
    let d1 = cross_2d(a, b, p);
    let d2 = cross_2d(b, c, p);
    let d3 = cross_2d(c, a, p);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_3d(positions: &[[f64; 3]], tris: &[[u32; 3]]) -> f64 {
        tris.iter()
            .map(|t| {
                let [a, b, c] = t.map(|v| positions[v as usize]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() * 0.5
            })
            .sum()
    }

    /// L-shape: a 2 x 2 square with the top-right 1 x 1 quarter removed, area 3.
    fn l_shape() -> (Vec<[f64; 3]>, Vec<u32>) {
        let p = vec![[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 2.0, 0.0]];
        // Start at (1, 2): a fan from there would spill into the missing quarter.
        (p, vec![4, 5, 0, 1, 2, 3])
    }

    #[test]
    fn concave_l_shape_covers_its_area_exactly() {
        let (p, ring) = l_shape();
        let tris = triangulate(&p, &ring);
        assert_eq!(tris.len(), 4);
        assert!((area_3d(&p, &tris) - 3.0).abs() < 1e-5);
        // Every triangle keeps the ring's counter-clockwise (+Z) winding.
        for t in &tris {
            let [a, b, c] = t.map(|v| p[v as usize]);
            assert!(cross_2d([a[0], a[1]], [b[0], b[1]], [c[0], c[1]]) > 0.0);
        }
    }

    #[test]
    fn works_on_vertical_and_clockwise_projected_faces() {
        // The same L standing in the XZ plane, wound so its normal points -Y.
        let (p, ring) = l_shape();
        let p: Vec<[f64; 3]> = p.iter().map(|q| [q[0], 0.0, q[1]]).collect();
        let tris = triangulate(&p, &ring);
        assert!((area_3d(&p, &tris) - 3.0).abs() < 1e-5);
    }

    #[test]
    fn collinear_points_still_give_n_minus_2_triangles() {
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [2.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let ring: Vec<u32> = (0..5).collect();
        let tris = triangulate(&p, &ring);
        assert_eq!(tris.len(), 3);
        assert!((area_3d(&p, &tris) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn a_square_with_a_square_hole_covers_the_ring_only() {
        // 4 x 4 square, a 2 x 2 hole in the middle: 16 - 4 = 12 square units
        let positions = vec![
            [0.0, 0.0, 0.0],
            [4.0, 0.0, 0.0],
            [4.0, 4.0, 0.0],
            [0.0, 4.0, 0.0],
            [1.0, 1.0, 0.0],
            [3.0, 1.0, 0.0],
            [3.0, 3.0, 0.0],
            [1.0, 3.0, 0.0],
        ];
        for hole in [vec![4, 7, 6, 5], vec![4, 5, 6, 7]] {
            let tris = triangulate_with_holes(&positions, &[0, 1, 2, 3], &[hole]);
            assert!((area_3d(&positions, &tris) - 12.0).abs() < 1e-9, "area {}", area_3d(&positions, &tris));
            for t in &tris {
                let c = [0, 1, 2].map(|k| t.iter().map(|&v| positions[v as usize][k]).sum::<f64>() / 3.0);
                assert!(!(c[0] > 1.0 && c[0] < 3.0 && c[1] > 1.0 && c[1] < 3.0), "no triangle in the hole: {t:?}");
            }
        }
    }

    #[test]
    fn two_holes_side_by_side() {
        // 6 x 2 wall, two 1 x 1 windows: 12 - 2 = 10
        let positions = vec![
            [0.0, 0.0, 0.0],
            [6.0, 0.0, 0.0],
            [6.0, 2.0, 0.0],
            [0.0, 2.0, 0.0],
            [1.0, 0.5, 0.0],
            [2.0, 0.5, 0.0],
            [2.0, 1.5, 0.0],
            [1.0, 1.5, 0.0],
            [4.0, 0.5, 0.0],
            [5.0, 0.5, 0.0],
            [5.0, 1.5, 0.0],
            [4.0, 1.5, 0.0],
        ];
        let tris = triangulate_with_holes(&positions, &[0, 1, 2, 3], &[vec![4, 7, 6, 5], vec![8, 11, 10, 9]]);
        assert!((area_3d(&positions, &tris) - 10.0).abs() < 1e-9, "area {}", area_3d(&positions, &tris));
    }

    /// `triangulate` as it was before big rings got `ears_fast`, kept word for word: what the faster clipping must match.
    fn old_triangulate(positions: &[[f64; 3]], ring: &[u32]) -> Vec<[u32; 3]> {
        let n = ring.len();
        if n < 3 {
            return Vec::new();
        }
        if n == 3 {
            return vec![[ring[0], ring[1], ring[2]]];
        }
        let normal = newell(positions, ring);
        let (ax, ay) = match dominant_axis(normal) {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let pts: Vec<[f64; 2]> = ring
            .iter()
            .map(|&v| {
                let p = positions[v as usize];
                [p[ax], p[ay]]
            })
            .collect();
        let orientation = signed_area_2d(&pts).signum();
        let mut idx: Vec<usize> = (0..n).collect();
        let mut out = Vec::with_capacity(n - 2);
        let mut guard = 0;
        while idx.len() > 3 && guard < n * n {
            guard += 1;
            let m = idx.len();
            let ear = (0..m).find(|&i| {
                let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                let turn = cross_2d(pts[a], pts[b], pts[c]) * orientation;
                if turn <= 0.0 {
                    return false;
                }
                !idx.iter().any(|&p| p != a && p != b && p != c && in_triangle(pts[p], pts[a], pts[b], pts[c]))
            });
            match ear {
                Some(i) => {
                    let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                    out.push([ring[a], ring[b], ring[c]]);
                    idx.remove(i);
                }
                None => break,
            }
        }
        for i in 1..idx.len() - 1 {
            out.push([ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]]);
        }
        out
    }

    /// `clip` as it was, word for word.
    fn old_clip(ring: &[u32], pts: &[[f64; 2]], orientation: f64) -> Vec<[u32; 3]> {
        let n = ring.len();
        let mut idx: Vec<usize> = (0..n).collect();
        let mut out = Vec::with_capacity(n.saturating_sub(2));
        let mut guard = 0;
        while idx.len() > 3 && guard < n * n {
            guard += 1;
            let m = idx.len();
            let ear = (0..m).find(|&i| {
                let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                let turn = cross_2d(pts[a], pts[b], pts[c]) * orientation;
                if turn <= 0.0 {
                    return false;
                }
                let corner = |p: usize| ring[p] == ring[a] || ring[p] == ring[b] || ring[p] == ring[c];
                !idx.iter().any(|&p| !corner(p) && in_triangle(pts[p], pts[a], pts[b], pts[c]))
            });
            match ear {
                Some(i) => {
                    let (a, b, c) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
                    out.push([ring[a], ring[b], ring[c]]);
                    idx.remove(i);
                }
                None => match (0..m).find(|&i| cross_2d(pts[idx[(i + m - 1) % m]], pts[idx[i]], pts[idx[(i + 1) % m]]).abs() < 1e-18) {
                    Some(i) => {
                        idx.remove(i);
                    }
                    None => break,
                },
            }
        }
        for i in 1..idx.len().saturating_sub(1) {
            let t = [ring[idx[0]], ring[idx[i]], ring[idx[i + 1]]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                out.push(t);
            }
        }
        out
    }

    /// Repeatable pseudo-random numbers (SplitMix64).
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        }

        /// In [0, 1).
        fn unit(&mut self) -> f64 {
            (self.next() >> 11) as f64 / (1u64 << 53) as f64
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// A star-shaped ring round the origin: `n` corners at increasing angles, distances between `1 - jag` and 1.
    fn star(rng: &mut Rng, n: usize, jag: f64) -> Vec<[f64; 2]> {
        let mut angles: Vec<f64> = (0..n).map(|_| rng.unit() * std::f64::consts::TAU).collect();
        angles.sort_by(f64::total_cmp);
        angles
            .iter()
            .map(|&a| {
                let r = 1.0 - jag * rng.unit();
                [r * a.cos(), r * a.sin()]
            })
            .collect()
    }

    /// Every side of `ring` cut into `k` pieces along its straight line (corners in a row, as SketchUp's split edges are).
    fn split_sides(ring: &[[f64; 2]], k: usize) -> Vec<[f64; 2]> {
        let mut out = Vec::new();
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            for j in 0..k {
                let t = j as f64 / k as f64;
                out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
            }
        }
        out
    }

    /// The rings to compare the two clippers on: simple ones and every kind of awkward one (corners in a row, touching,
    /// repeated, self-crossing, slivers, far from the origin, tiny, huge, not numbers). Each is its points and the ring of
    /// vertex numbers into them.
    fn awkward_rings() -> Vec<(Vec<[f64; 2]>, Vec<u32>)> {
        let mut rng = Rng(0x00DA_EDA1);
        let mut cases: Vec<Vec<[f64; 2]>> = Vec::new();
        for n in [3, 4, 5, 8, 20, 47, 48, 49, 64, 100, 150, 300] {
            cases.push(star(&mut rng, n, 0.0));
            cases.push(star(&mut rng, n, 0.6));
            cases.push(star(&mut rng, n, 0.95));
        }
        for (n, k) in [(4, 30), (12, 8), (40, 3), (90, 2)] {
            let base = star(&mut rng, n, 0.5);
            cases.push(split_sides(&base, k));
            // an axis-aligned rectangle cut up: corners exactly in a row
            let w = 1.0 + rng.unit();
            cases.push(split_sides(&[[0.0, 0.0], [w, 0.0], [w, 1.0], [0.0, 1.0]], n));
        }
        for n in [6, 30, 60, 120] {
            // self-crossing scribbles
            cases.push((0..n).map(|_| [rng.unit(), rng.unit()]).collect());
            // on a grid, many corners in rows and columns and some at the same place
            cases.push((0..n).map(|_| [rng.below(6) as f64, rng.below(6) as f64]).collect());
        }
        // all in a row, all at one point, slivers a hair off a line
        cases.push((0..60).map(|i| [i as f64 * 0.1, i as f64 * 0.3]).collect());
        cases.push(vec![[0.25, 0.5]; 50]);
        cases.push((0..80).map(|i| [i as f64, (i % 2) as f64 * 1e-13 + i as f64 * 0.5]).collect());
        cases.push((0..80).map(|i| [i as f64 * 0.37, 1e-9 * (i as f64).sin()]).collect());
        let mut more = Vec::new();
        for c in &cases {
            if c.len() >= 40 {
                // far from the origin, tiny, huge, turned
                more.push(c.iter().map(|p| [p[0] + 3.0e6, p[1] - 7.0e5]).collect::<Vec<_>>());
                more.push(c.iter().map(|p| [p[0] * 1e-9, p[1] * 1e-9]).collect());
                more.push(c.iter().map(|p| [p[0] * 1e40, p[1] * 1e40]).collect());
                let (s, k) = (0.3f64.sin(), 0.3f64.cos());
                more.push(c.iter().map(|p| [k * p[0] - s * p[1], s * p[0] + k * p[1]]).collect());
            }
        }
        cases.extend(more);
        let mut rings: Vec<(Vec<[f64; 2]>, Vec<u32>)> = cases
            .into_iter()
            .map(|p| {
                let ring = (0..p.len() as u32).collect();
                (p, ring)
            })
            .collect();
        // one corner made not a number; one infinitely far
        let (mut p, ring) = rings[20].clone();
        p[7][0] = f64::NAN;
        rings.push((p, ring.clone()));
        let (mut p, ring) = rings[21].clone();
        p[3][1] = f64::INFINITY;
        rings.push((p, ring));
        // a ring through one vertex twice (two loops touching there), and corners repeated at once
        for n in [30, 80] {
            let mut pts = star(&mut rng, n, 0.4);
            let other: Vec<[f64; 2]> = star(&mut rng, n, 0.4).into_iter().map(|q| [q[0] + 2.0, q[1]]).collect();
            pts.extend(other);
            let mut ring: Vec<u32> = (0..n as u32).collect();
            ring.push(0);
            ring.extend(n as u32..2 * n as u32);
            rings.push((pts.clone(), ring));
            let ring: Vec<u32> = (0..2 * n as u32).flat_map(|i| if i % 7 == 0 { vec![i, i] } else { vec![i] }).collect();
            rings.push((pts, ring));
        }
        rings
    }

    #[test]
    fn the_fast_clipper_cuts_exactly_the_plain_ones_triangles() {
        for (positions, ring) in awkward_rings() {
            let pts: Vec<[f64; 2]> = ring.iter().map(|&v| positions[v as usize]).collect();
            let around = signed_area_2d(&pts).signum();
            for kind in [Ring::Outline, Ring::Bridged] {
                for orientation in [around, 1.0, -1.0] {
                    let plain = ears_plain(&ring, &pts, orientation, kind);
                    let fast = ears_fast(&ring, &pts, orientation, kind);
                    assert_eq!(fast, plain, "{} corners, {kind:?}, orientation {orientation}", ring.len());
                }
            }
        }
    }

    #[test]
    fn faces_triangulate_as_they_did() {
        // the awkward rings stood up in 3D on planes facing every way
        let mut rng = Rng(7);
        for (flat, ring) in awkward_rings() {
            let n = [rng.unit() - 0.5, rng.unit() - 0.5, rng.unit() - 0.5];
            let u = [n[1], -n[0], 0.0];
            let v = [n[1] * u[2] - n[2] * u[1], n[2] * u[0] - n[0] * u[2], n[0] * u[1] - n[1] * u[0]];
            for frame in [[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0]], [u, v]] {
                let positions: Vec<[f64; 3]> =
                    flat.iter().map(|p| std::array::from_fn(|k| frame[0][k] * p[0] + frame[1][k] * p[1])).collect();
                assert_eq!(triangulate(&positions, &ring), old_triangulate(&positions, &ring), "{} corners", ring.len());
            }
        }
    }

    #[test]
    fn faces_with_holes_triangulate_as_they_did() {
        let mut rng = Rng(11);
        for (outline, holes, size) in [(60, 1, 50), (200, 1, 180), (90, 6, 12), (40, 25, 5), (300, 3, 60)] {
            for _ in 0..3 {
                // an outline round the origin and holes inside it, apart from each other, some cut along a line
                let mut positions: Vec<[f64; 3]> =
                    star(&mut rng, outline, 0.05).into_iter().map(|p| [p[0] * 10.0, p[1] * 10.0, 0.0]).collect();
                let outer: Vec<u32> = (0..outline as u32).collect();
                let mut hole_rings = Vec::new();
                for h in 0..holes {
                    let (cx, cy) = (-6.0 + 12.0 * (h as f64 + 0.5) / holes as f64, 0.0);
                    let r = (5.0 / holes as f64).min(3.0);
                    let shape =
                        if h % 2 == 0 { star(&mut rng, size, 0.3) } else { split_sides(&star(&mut rng, size.div_ceil(3).max(3), 0.3), 3) };
                    let from = positions.len() as u32;
                    positions.extend(shape.iter().rev().map(|p| [cx + p[0] * r, cy + p[1] * r, 0.0]));
                    hole_rings.push((from..positions.len() as u32).collect::<Vec<u32>>());
                }
                for tilt in [None, Some(0.7)] {
                    let placed: Vec<[f64; 3]> = match tilt {
                        None => positions.clone(),
                        Some(t) => positions.iter().map(|p| [p[0], p[1] * f64::cos(t), p[1] * f64::sin(t) + 4.0]).collect(),
                    };
                    let (ring, pts) = bridged(&placed, &outer, &hole_rings);
                    assert_eq!(clip(&ring, &pts, 1.0), old_clip(&ring, &pts, 1.0), "{outline} + {holes} x {size}");
                    let tris = triangulate_with_holes(&placed, &outer, &hole_rings);
                    assert_eq!(tris, old_clip(&ring, &pts, 1.0));
                }
            }
        }
    }

    #[test]
    fn a_big_face_with_a_big_hole_still_covers_its_area() {
        // a 2000-corner circle with a 1500-corner circle cut out (the shape of a round floor round a round well)
        let mut positions = Vec::new();
        for (n, r) in [(2000, 10.0), (1500, 6.0)] {
            for i in 0..n {
                let a = i as f64 / n as f64 * std::f64::consts::TAU;
                positions.push([r * a.cos(), r * a.sin(), 0.0]);
            }
        }
        let outer: Vec<u32> = (0..2000).collect();
        let hole: Vec<u32> = (2000..3500).rev().collect();
        let tris = triangulate_with_holes(&positions, &outer, &[hole]);
        assert_eq!(tris.len(), 3500, "every corner of a ring with one hole gives a triangle");
        let polygon = |n: f64, r: f64| 0.5 * n * r * r * (std::f64::consts::TAU / n).sin();
        let want = polygon(2000.0, 10.0) - polygon(1500.0, 6.0);
        assert!((area_3d(&positions, &tris) - want).abs() < 1e-9 * want, "{} against {want}", area_3d(&positions, &tris));
    }
}
