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
    // Border colour in rgb; alpha is 1 when the camera is a circle and 0 for a rounded rectangle.
    // Border colour in rgb; alpha picks the drawn shape: 0 plain rectangle, 1 rounded rectangle,
    // 2 circle, for both the camera and the background.
    float4 border_color;
    // x: corner radius in pixels, y: border width in pixels, z: output width, w: output height.
    float4 shape;
    // YCbCr to RGB: red from Cr, green from Cb, green from Cr, blue from Cb.
    float4 yuv_coef;
    // x: luma offset, y: luma scale, z: chroma offset, w: chroma scale.
    float4 yuv_range;
    // Key colour in rgb 0 to 1; w is the key mode: 0 off, 1 keyed, 2 matte view.
    float4 key_color;
    // x: similarity, y: smoothness, z: spill, w: 1 when the mask rectangle is on.
    float4 key_params;
    // Left, top, right and bottom of the part of the camera frame the garbage crop keeps, 0 to 1.
    float4 key_garbage;
    // Left, top, right and bottom of the mask rectangle, 0 to 1.
    float4 key_mask;
};

// Scales the chroma distance so the similarity and smoothness sliders act like OBS's: OBS measures Cb and Cr
// in the limited range, where they reach 0.439, and this shader measures them over the full range up to 0.5.
static const float KEY_DISTANCE_SCALE = 0.878;

// How far past the similarity the spill suppression fades out, in the same units as the distance.
static const float KEY_SPILL_REACH = 0.35;

Texture2D<float4> tex_bgra : register(t0);
Texture2D<float> tex_y : register(t1);
Texture2D<float2> tex_uv : register(t2);
SamplerState samp : register(s0);

struct VsOut
{
    float4 pos : SV_Position;
    float2 px : TEXCOORD0;
};

// Draws the rectangle, one pixel larger on every side so the anti-aliased edge has room. A circle's
// border ring lies outside the rectangle, so the quad also grows by the border width.
// Use Draw(4, 0) with a triangle strip.
VsOut vs_main(uint id : SV_VertexID)
{
    float2 corner = float2((float)(id & 1u), (float)(id >> 1u));
    float grow = 1.0 + (border_color.w > 0.5 ? shape.y : 0.0);
    float2 lo = dst_rect.xy - grow;
    float2 hi = dst_rect.zw + grow;
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

// Draws a BGRA texture into the rectangle. border_color.w picks the shape that cuts the texture:
// 0 the plain rectangle, 1 a rounded rectangle of shape.x radius, 2 a circle.
float4 ps_bgra(VsOut i) : SV_Target
{
    float coverage;
    if (border_color.w > 1.5)
    {
        float2 centre = (dst_rect.xy + dst_rect.zw) * 0.5;
        float radius = (dst_rect.z - dst_rect.x) * 0.5;
        coverage = saturate(0.5 - (length(i.px - centre) - radius));
    }
    else if (border_color.w > 0.5)
    {
        coverage = saturate(0.5 - rect_distance(i.px, shape.x));
    }
    else
    {
        coverage = saturate(0.5 - rect_distance(i.px, 0.0));
    }
    float3 rgb = tex_bgra.SampleLevel(samp, source_uv(i.px), 0).rgb;
    return float4(rgb * coverage, coverage);
}

// Cb and Cr of an rgb colour, each from -0.5 to 0.5, with the camera's matrix. The matrix weights come
// from the YCbCr to RGB constants: red from Cr is 2 (1 - kr) and blue from Cb is 2 (1 - kb).
float2 rgb_to_cbcr(float3 rgb)
{
    float kr = 1.0 - yuv_coef.x * 0.5;
    float kb = 1.0 - yuv_coef.w * 0.5;
    float luma = dot(rgb, float3(kr, 1.0 - kr - kb, kb));
    return float2((rgb.z - luma) / yuv_coef.w, (rgb.x - luma) / yuv_coef.x);
}

// Draws an NV12 camera texture, as a Y plane and a UV plane, into the rectangle with rounded
// corners and a border, both with anti-aliased edges. When the key is on, the key colour becomes
// transparent: alpha is a smoothstep of the Cb Cr distance to the key colour, the spill of the key
// colour is pulled out of the pixel, and the garbage crop and the mask rectangle force alpha to 0.
// In the matte view the picture is the alpha as grey instead.
float4 ps_nv12(VsOut i) : SV_Target
{
    float2 uv = source_uv(i.px);
    float y = tex_y.SampleLevel(samp, uv, 0);
    float2 c = tex_uv.SampleLevel(samp, uv, 0);
    float luma = (y - yuv_range.x) * yuv_range.y;
    float cb = (c.x - yuv_range.z) * yuv_range.w;
    float cr = (c.y - yuv_range.z) * yuv_range.w;

    float key_alpha = 1.0;
    if (key_color.w > 0.5)
    {
        float2 chroma = float2(cb, cr);
        float2 key_chroma = rgb_to_cbcr(key_color.rgb);
        float dist = distance(chroma, key_chroma) * KEY_DISTANCE_SCALE;
        float similarity = key_params.x;
        key_alpha = smoothstep(similarity, similarity + max(key_params.y, 0.0001), dist);

        // Spill: take the part of the chroma that points the way of the key colour out of the pixel, more the
        // higher the spill slider and the closer the pixel is to the key colour.
        float key_len = length(key_chroma);
        if (key_len > 0.0001)
        {
            float2 dir = key_chroma / key_len;
            float along = max(dot(chroma, dir), 0.0);
            float reach = 1.0 - smoothstep(similarity, similarity + KEY_SPILL_REACH, dist);
            chroma -= dir * along * saturate(key_params.z * 4.0) * reach;
            cb = chroma.x;
            cr = chroma.y;
        }

        if (any(uv < key_garbage.xy) || any(uv > key_garbage.zw))
        {
            key_alpha = 0.0;
        }
        if (key_params.w > 0.5 && all(uv >= key_mask.xy) && all(uv <= key_mask.zw))
        {
            key_alpha = 0.0;
        }
    }

    float3 rgb = saturate(float3(luma + yuv_coef.x * cr, luma - yuv_coef.y * cb - yuv_coef.z * cr, luma + yuv_coef.w * cb));
    // The picture and how much of it shows; the matte view shows the alpha as grey over the whole shape.
    float3 picture_rgb = rgb;
    float picture_alpha = key_alpha;
    if (key_color.w > 1.5)
    {
        picture_rgb = float3(key_alpha, key_alpha, key_alpha);
        picture_alpha = 1.0;
    }

    float border = shape.y;
    if (border_color.w > 0.5)
    {
        // A circle: the picture fills the circle and the border is a ring outside it.
        float2 centre = (dst_rect.xy + dst_rect.zw) * 0.5;
        float radius = (dst_rect.z - dst_rect.x) * 0.5;
        float dc = length(i.px - centre) - radius;
        float inside = saturate(0.5 - dc);
        float ring = saturate(0.5 - (dc - border));
        float picture = inside * picture_alpha;
        float edge = ring - inside;
        return float4(picture_rgb * picture + border_color.rgb * edge, picture + edge);
    }

    float d = rect_distance(i.px, shape.x);
    float outer = saturate(0.5 - d);
    float inner = border > 0.0 ? saturate(0.5 - (d + border)) : 1.0;
    float picture = inner * picture_alpha;
    float edge = 1.0 - inner;
    return float4((picture_rgb * picture + border_color.rgb * edge) * outer, (picture + edge) * outer);
}
