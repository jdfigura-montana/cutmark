// Rectangles drawn one over another: pictures from an atlas, rounded boxes (a circle or a ring is
// one with the largest corner radius) and the spotlight. Colours are premultiplied sRGB values.

struct Target {
    // The target's size (xy) and the atlas's side (z).
    size: vec4<f32>,
}

@group(0) @binding(0) var<uniform> target_info: Target;
@group(0) @binding(1) var atlas: texture_2d<f32>;
@group(0) @binding(2) var linear_sampler: sampler;

const PICTURE: u32 = 0u;
const BOX: u32 = 1u;
const SPOTLIGHT: u32 = 2u;

struct Instance {
    // The rectangle in target pixels: top left (xy) and size (zw).
    @location(0) rect: vec4<f32>,
    // A picture's part of the atlas in texels: top left (xy) and size (zw). The spotlight's
    // centre in target pixels (xy).
    @location(1) source: vec4<f32>,
    @location(2) color: vec4<f32>,
    // The kind (x). A box: the corner radius (y) and the line width, 0 for a filled one (z). The
    // spotlight: the radius (y) and the width of its soft edge (z).
    @location(3) params: vec4<f32>,
}

struct Vertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) rect: vec4<f32>,
    @location(2) @interpolate(flat) source: vec4<f32>,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) params: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32, instance: Instance) -> Vertex {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    // A box's soft edge reaches a pixel past its rectangle.
    let grow = select(0.0, 1.0, u32(instance.params.x) == BOX);
    let point = instance.rect.xy - vec2<f32>(grow) + corner * (instance.rect.zw + vec2<f32>(2.0 * grow));
    let at = point / target_info.size.xy;
    var out: Vertex;
    out.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = (instance.source.xy + corner * instance.source.zw) / target_info.size.z;
    out.rect = instance.rect;
    out.source = instance.source;
    out.color = instance.color;
    out.params = instance.params;
    return out;
}

@fragment
fn fragment(in: Vertex) -> @location(0) vec4<f32> {
    let kind = u32(in.params.x);
    if kind == PICTURE {
        return textureSampleLevel(atlas, linear_sampler, in.uv, 0.0) * in.color;
    }
    let point = in.position.xy;
    if kind == SPOTLIGHT {
        let half_edge = max(in.params.z, 1.0) * 0.5;
        let away = length(point - in.source.xy);
        return in.color * smoothstep(in.params.y - half_edge, in.params.y + half_edge, away);
    }
    // The signed distance to the rounded rectangle: negative inside.
    let half_size = in.rect.zw * 0.5;
    let radius = min(in.params.y, min(half_size.x, half_size.y));
    let q = abs(point - in.rect.xy - half_size) - half_size + vec2<f32>(radius);
    let away = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
    var coverage = clamp(0.5 - away, 0.0, 1.0);
    if in.params.z > 0.0 {
        // A line inside the edge: what the box covers less what its inside covers.
        coverage -= clamp(0.5 - away - in.params.z, 0.0, 1.0);
    }
    return in.color * coverage;
}
