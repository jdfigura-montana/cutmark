// The frame pass: the background, the shadow, the picture with rounded corners on top, and last
// the shortcut overlay. Colours are mixed as their sRGB values, as image editors do.

struct Frame {
    // The picture's place in output pixels: top left (xy) and size (zw).
    content: vec4<f32>,
    // The gradient's top left and bottom right colours. A solid background has both the same.
    color_a: vec4<f32>,
    color_b: vec4<f32>,
    // The part of the background picture that covers the output: offset (xy) and size (zw).
    image: vec4<f32>,
    // The output size (xy), the corner radius (z), and 1 for a picture background (w).
    output: vec4<f32>,
    // The shadow's blur (x), downward offset (y) and opacity (z), and 1 when the target encodes
    // sRGB by itself (w).
    shadow: vec4<f32>,
    // The key caps' place in output pixels: top left (xy) and size (zw), all 0 without any.
    keys: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var backdrop: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
// Premultiplied, at its own size.
@group(0) @binding(4) var key_caps: texture_2d<f32>;

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - corner.y * 2.0, 0.0, 1.0);
}

// The signed distance to the picture's rounded rectangle moved by `shift`: negative inside.
fn distance_to_picture(point: vec2<f32>, shift: vec2<f32>) -> f32 {
    let half_size = frame.content.zw * 0.5;
    let radius = frame.output.z;
    let from_center = abs(point - frame.content.xy - half_size - shift);
    let q = from_center - half_size + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let point = position.xy;
    let size = frame.output.xy;

    // 0 in the top left corner, 1 in the bottom right one.
    let along = clamp(dot(point, size) / dot(size, size), 0.0, 1.0);
    var color = mix(frame.color_a.rgb, frame.color_b.rgb, along);
    let image_uv = frame.image.xy + point / size * frame.image.zw;
    let image_color = textureSampleLevel(backdrop, linear_sampler, image_uv, 0.0).rgb;
    color = select(color, image_color, frame.output.w > 0.5);

    if frame.shadow.z > 0.0 {
        let away = distance_to_picture(point, vec2<f32>(0.0, frame.shadow.y));
        let blur = max(frame.shadow.x, 0.5);
        color = color * (1.0 - frame.shadow.z * (1.0 - smoothstep(-blur, blur, away)));
    }

    // The picture was rendered at its own size, so each of its pixels is read as it is.
    let last = vec2<i32>(textureDimensions(picture)) - vec2<i32>(1);
    let texel = clamp(vec2<i32>(floor(point - frame.content.xy)), vec2<i32>(0), last);
    let coverage = clamp(0.5 - distance_to_picture(point, vec2<f32>(0.0)), 0.0, 1.0);
    color = mix(color, textureLoad(picture, texel, 0).rgb, coverage);

    let in_keys = point - frame.keys.xy;
    if all(in_keys >= vec2<f32>(0.0)) && all(in_keys < frame.keys.zw) {
        let cap = textureLoad(key_caps, vec2<i32>(floor(in_keys)), 0);
        color = color * (1.0 - cap.a) + cap.rgb;
    }

    if frame.shadow.w > 0.5 {
        color = to_linear(color);
    }
    return vec4<f32>(color, 1.0);
}
