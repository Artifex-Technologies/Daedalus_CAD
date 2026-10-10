struct Uniforms {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) highlight: f32,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) highlight: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.normal = in.normal;
    out.highlight = in.highlight;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // Two-sided lighting: SketchUp-style models are often viewed from the back.
    let d = abs(dot(normalize(in.normal), normalize(u.light.xyz)));
    let shade = 0.30 + 0.70 * d;
    let base = vec3<f32>(0.78, 0.80, 0.84);
    let selected = vec3<f32>(0.30, 0.55, 0.95);
    let flagged = vec3<f32>(0.90, 0.25, 0.22);
    // highlight: 0 = normal, 1 = selected, 2 = flagged by the solid inspector
    let tint = select(selected, flagged, in.highlight > 1.5);
    return vec4<f32>(mix(base, tint, min(in.highlight, 1.0)) * shade, 1.0);
}
