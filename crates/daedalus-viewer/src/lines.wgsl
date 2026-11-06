struct Uniforms {
    view_proj: mat4x4<f32>,
    light: vec4<f32>,
    // x: face style (0 shaded, 1 hidden line, 2 X-ray)
    style: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip = u.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}

// Back Edges: grey dashes (the dash follows the screen position)
@fragment
fn fs_dashed(in: VertexOut) -> @location(0) vec4<f32> {
    let k = floor((in.clip.x + in.clip.y) / 5.0);
    if (k - 2.0 * floor(k / 2.0) > 0.5) {
        discard;
    }
    return vec4<f32>(0.45, 0.46, 0.5, 1.0);
}
