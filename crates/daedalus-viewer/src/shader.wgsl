struct Uniforms {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // x: face style (0 shaded, 1 hidden line, 2 X-ray)
    style: vec4<f32>,
    // Section Plane: xyz the normal of the side cut away, w the offset; all zero when there is none
    section: vec4<f32>,
};

// whether a world point lies on the side the section plane cuts away
fn cut_away(p: vec3<f32>) -> bool {
    return dot(u.section.xyz, u.section.xyz) > 0.5 && dot(p, u.section.xyz) > u.section.w;
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) highlight: f32,
    @location(3) color: vec3<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) highlight: f32,
    @location(2) color: vec3<f32>,
    @location(3) world: vec3<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.highlight = in.highlight;
    out.color = in.color;
    out.world = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if (cut_away(in.world)) {
        discard;
    }
    // Two-sided lighting: SketchUp-style models are often viewed from the back.
    let d = abs(dot(normalize(in.normal), normalize(u.light.xyz)));
    let shade = 0.30 + 0.70 * d;
    // the face's paint, or the default material in the Monochrome style
    let base = select(in.color, vec3<f32>(0.78, 0.80, 0.84), u.style.x > 2.5);
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
    if (u.style.x > 0.5) {
        // hidden line: faces in the background colour, unlit (a selection still shows)
        return vec4<f32>(mix(paper, tint, min(in.highlight, 1.0) * 0.5), 1.0);
    }
    return vec4<f32>(lit, 1.0);
}
