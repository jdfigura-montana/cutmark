// Pass 1: the source's view rectangle, stretched over the whole target with linear filtering.

struct Zoom {
    // The view rectangle in texture coordinates: offset (xy) and size (zw).
    view: vec4<f32>,
}

@group(0) @binding(0) var<uniform> zoom: Zoom;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle that covers the target.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Vertex {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: Vertex;
    out.position = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
    out.uv = zoom.view.xy + corner * zoom.view.zw;
    return out;
}

@fragment
fn fragment(in: Vertex) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(source, linear_sampler, in.uv, 0.0).rgb, 1.0);
}
