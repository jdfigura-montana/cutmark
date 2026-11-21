// Compositor shaders. Compiled at startup with D3DCompile (vs_5_0 and ps_5_0).
//
// One quad per picture: the vertex shader places a rectangle in output pixels, and the pixel
// shaders fill it. Output is premultiplied alpha, blended with ONE and INV_SRC_ALPHA.

cbuffer Params : register(b0)
{
    // Left, top, right and bottom of the rectangle in output pixels.
    float4 dst_rect;
    // Left, top, right and bottom of the part of the source texture that is shown, 0 to 1.
    float4 src_uv;
    // Border colour in rgb; alpha is unused.
    float4 border_color;
    // x: corner radius in pixels, y: border width in pixels, z: output width, w: output height.
    float4 shape;
    // YCbCr to RGB: red from Cr, green from Cb, green from Cr, blue from Cb.
    float4 yuv_coef;
    // x: luma offset, y: luma scale, z: chroma offset, w: chroma scale.
    float4 yuv_range;
};

Texture2D<float4> tex_bgra : register(t0);
Texture2D<float> tex_y : register(t1);
Texture2D<float2> tex_uv : register(t2);
SamplerState samp : register(s0);

struct VsOut
{
    float4 pos : SV_Position;
    float2 px : TEXCOORD0;
};

// Draws the rectangle, one pixel larger on every side so the anti-aliased edge has room.
// Use Draw(4, 0) with a triangle strip.
VsOut vs_main(uint id : SV_VertexID)
{
    float2 corner = float2((float)(id & 1u), (float)(id >> 1u));
    float2 lo = dst_rect.xy - 1.0;
    float2 hi = dst_rect.zw + 1.0;
    float2 px = lerp(lo, hi, corner);
    VsOut o;
    o.px = px;
    o.pos = float4(px.x / shape.z * 2.0 - 1.0, 1.0 - px.y / shape.w * 2.0, 0.0, 1.0);
    return o;
}

// Signed distance from a pixel to the edge of the rounded rectangle: negative inside.
float rect_distance(float2 p, float radius)
{
    float2 centre = (dst_rect.xy + dst_rect.zw) * 0.5;
    float2 half_size = (dst_rect.zw - dst_rect.xy) * 0.5;
    float r = min(radius, min(half_size.x, half_size.y));
    float2 q = abs(p - centre) - (half_size - r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

// The texture coordinate of a pixel inside the rectangle.
float2 source_uv(float2 p)
{
    float2 t = saturate((p - dst_rect.xy) / (dst_rect.zw - dst_rect.xy));
    return lerp(src_uv.xy, src_uv.zw, t);
}

// Draws a BGRA texture into the rectangle.
float4 ps_bgra(VsOut i) : SV_Target
{
    float coverage = saturate(0.5 - rect_distance(i.px, 0.0));
    float3 rgb = tex_bgra.SampleLevel(samp, source_uv(i.px), 0).rgb;
    return float4(rgb * coverage, coverage);
}

// Draws an NV12 camera texture, as a Y plane and a UV plane, into the rectangle with rounded
// corners and a border, both with anti-aliased edges.
float4 ps_nv12(VsOut i) : SV_Target
{
    float2 uv = source_uv(i.px);
    float y = tex_y.SampleLevel(samp, uv, 0);
    float2 c = tex_uv.SampleLevel(samp, uv, 0);
    float luma = (y - yuv_range.x) * yuv_range.y;
    float cb = (c.x - yuv_range.z) * yuv_range.w;
    float cr = (c.y - yuv_range.z) * yuv_range.w;
    float3 rgb = saturate(float3(luma + yuv_coef.x * cr, luma - yuv_coef.y * cb - yuv_coef.z * cr, luma + yuv_coef.w * cb));

    float d = rect_distance(i.px, shape.x);
    float outer = saturate(0.5 - d);
    float border = shape.y;
    float inner = border > 0.0 ? saturate(0.5 - (d + border)) : 1.0;
    float3 colour = lerp(border_color.rgb, rgb, inner);
    return float4(colour * outer, outer);
}
