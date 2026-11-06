struct Uniforms {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // x: face style (0 shaded, 1 hidden line, 2 X-ray); y, z: viewport size in pixels; w: line width in pixels
    style: vec4<f32>,
    // Section Plane: xyz the normal of the side cut away, w the offset; all zero when there is none
    section: vec4<f32>,
};

// whether a world point lies on the side the section plane cuts away
fn cut_away(p: vec3<f32>) -> bool {
    return dot(u.section.xyz, u.section.xyz) > 0.5 && dot(p, u.section.xyz) > u.section.w;
}

@group(0) @binding(0) var<uniform> u: Uniforms;

// Lines are drawn as thin strips facing the screen, a little wider than the line, whose sides fade out: straight,
// smooth edges of an even width at any angle (GPU line primitives are one pixel wide and break into steps).
// One instance per segment, six vertices (two triangles) each.
struct SegmentIn {
    @builtin(vertex_index) corner: u32,
    @location(0) a: vec3<f32>,
    @location(1) color_a: vec4<f32>,
    @location(2) b: vec3<f32>,
    @location(3) color_b: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    // distance from the middle of the line, in pixels
    @location(1) across: f32,
    @location(2) world: vec3<f32>,
};

@vertex
fn vs_main(in: SegmentIn) -> VertexOut {
    var pa = u.view_proj * vec4<f32>(in.a, 1.0);
    var pb = u.view_proj * vec4<f32>(in.b, 1.0);
    var out: VertexOut;
    // cut the segment at the near plane (z = 0 in clip space), or drop it when it lies wholly behind the eye
    if (pa.z < 0.0 && pb.z < 0.0) {
        out.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        out.color = vec4<f32>(0.0);
        out.across = 0.0;
        out.world = vec3<f32>(0.0);
        return out;
    }
    if (pa.z < 0.0) {
        pa = mix(pa, pb, pa.z / (pa.z - pb.z));
    } else if (pb.z < 0.0) {
        pb = mix(pb, pa, pb.z / (pb.z - pa.z));
    }
    let vp = u.style.yz;
    let sa = pa.xy / pa.w * 0.5 * vp;
    let sb = pb.xy / pb.w * 0.5 * vp;
    var dir = sb - sa;
    if (length(dir) < 1e-4) {
        dir = vec2<f32>(1.0, 0.0);
    }
    dir = normalize(dir);
    let normal = vec2<f32>(-dir.y, dir.x);
    // which corner: (end, side)
    var ends = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
    var sides = array<f32, 6>(-1.0, -1.0, 1.0, -1.0, 1.0, 1.0);
    let end = ends[in.corner];
    let side = sides[in.corner];
    let half = u.style.w * 0.5 + 1.0;
    var p = select(pa, pb, end > 0.5);
    // sideways by half the width (plus a pixel to fade over), and a little past each end so corners meet
    let along = select(-1.0, 1.0, end > 0.5);
    let offset = (normal * side + dir * along * 0.5) * half;
    p = vec4<f32>(p.xy + offset / (0.5 * vp) * p.w, p.z, p.w);
    // a hair toward the eye (about 0.2 % of the distance), so an edge always wins over the faces it borders
    p.z = p.z - 3e-5 * p.w;
    out.clip = p;
    out.color = mix(in.color_a, in.color_b, end);
    out.across = side * half;
    out.world = mix(in.a, in.b, end);
    return out;
}

// the coverage of a pixel this far from the middle: full inside the line, fading to nothing over one pixel
fn coverage(across: f32) -> f32 {
    return clamp(u.style.w * 0.5 + 0.5 - abs(across), 0.0, 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    let a = coverage(in.across);
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * a);
}

// Back Edges: grey dashes (the dash follows the screen position)
@fragment
fn fs_dashed(in: VertexOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    let k = floor((in.clip.x + in.clip.y) / 5.0);
    if (k - 2.0 * floor(k / 2.0) > 0.5) {
        discard;
    }
    let a = coverage(in.across);
    if (a <= 0.0) {
        discard;
    }
    return vec4<f32>(0.45, 0.46, 0.5, a);
}
