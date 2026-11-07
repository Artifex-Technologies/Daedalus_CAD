struct Uniforms {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // x: face style (0 shaded, 1 hidden line, 2 X-ray, 3 monochrome, 4 shaded with textures)
    style: vec4<f32>,
    // Section Plane: xyz the normal of the side cut away, w the offset; all zero when there is none
    section: vec4<f32>,
    // the eye (w = 1), or the direction toward it in parallel projection (w = 0)
    eye: vec4<f32>,
};

// whether a world point lies on the side the section plane cuts away
fn cut_away(p: vec3<f32>) -> bool {
    return dot(u.section.xyz, u.section.xyz) > 0.5 && dot(p, u.section.xyz) > u.section.w;
}

@group(0) @binding(0) var<uniform> u: Uniforms;
// the textures faces are painted with, all the same size, one layer each
@group(0) @binding(1) var textures: texture_2d_array<f32>;
@group(0) @binding(2) var tex_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) highlight: f32,
    @location(3) color: vec3<f32>,
    // where on its texture (in tiles: it repeats), and which layer (-1: none)
    @location(4) uv: vec2<f32>,
    @location(5) layer: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) highlight: f32,
    @location(2) color: vec3<f32>,
    @location(3) world: vec3<f32>,
    @location(4) uv: vec2<f32>,
    @location(5) layer: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.highlight = in.highlight;
    out.color = in.color;
    out.world = in.position;
    out.uv = in.uv;
    out.layer = in.layer;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // sampled first, while every pixel still runs (a texture lookup needs its neighbours)
    let texel = textureSample(textures, tex_sampler, in.uv, max(i32(in.layer + 0.5), 0)).rgb;
    if (cut_away(in.world)) {
        discard;
    }
    // section fill: with a cut, the inside of a solid shows through it; its faces are seen from behind and drawn in a flat
    // dark grey, so the cut reads as solid (not in X-ray)
    let to_eye = select(u.eye.xyz, u.eye.xyz - in.world, u.eye.w > 0.5);
    let sectioned = dot(u.section.xyz, u.section.xyz) > 0.5;
    if (sectioned && dot(in.normal, to_eye) < 0.0 && (u.style.x < 1.5 || u.style.x > 2.5)) {
        return vec4<f32>(0.24, 0.24, 0.27, 1.0);
    }
    // Two-sided lighting: SketchUp-style models are often viewed from the back.
    let d = abs(dot(normalize(in.normal), normalize(u.light.xyz)));
    let shade = 0.30 + 0.70 * d;
    // the face's paint, or the default material in the Monochrome style
    // with textures: the face's texture where it has one
    let painted = select(in.color, texel, u.style.x > 3.5 && in.layer > -0.5);
    let base = select(painted, vec3<f32>(0.78, 0.80, 0.84), u.style.x > 2.5 && u.style.x < 3.5);
    let selected = vec3<f32>(0.30, 0.55, 0.95);
    let flagged = vec3<f32>(0.90, 0.25, 0.22);
    let paper = vec3<f32>(0.93, 0.94, 0.96);
    // highlight 3: outside the group or component being edited, faded toward the background
    if (in.highlight > 2.5) {
        if (u.style.x > 0.5 && u.style.x < 1.5) {
            return vec4<f32>(paper, 1.0);
        }
        return vec4<f32>(mix(base * shade, paper, 0.6), select(1.0, 0.45, u.style.x > 1.5 && u.style.x < 2.5));
    }
    // highlight: 0 = normal, 1 = selected, 2 = flagged by the solid inspector
    let tint = select(selected, flagged, in.highlight > 1.5);
    let lit = mix(base, tint, min(in.highlight, 1.0)) * shade;
    if (u.style.x > 1.5 && u.style.x < 2.5) {
        // X-ray: see-through
        return vec4<f32>(lit, 0.45);
    }
    if (u.style.x > 0.5 && u.style.x < 1.5) {
        // hidden line: faces in the background colour, unlit (a selection still shows)
        return vec4<f32>(mix(paper, tint, min(in.highlight, 1.0) * 0.5), 1.0);
    }
    return vec4<f32>(lit, 1.0);
}
