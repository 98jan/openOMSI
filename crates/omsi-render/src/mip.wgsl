// Mipmap generation: each texel of the smaller level is the average of exactly the area of
// the bigger level it covers. For even sizes that is a 2x2 block; for odd ones (15 texels ->
// 7) it is a bit more than two texels a side, the outer ones with partial weights - a plain
// 2x2 block there shifts the picture and drops its last row, and display digits grew bolder
// each level and lost their bottom row.
//
// `fs_main` alpha-weights RGB so bright RGB stored in transparent texels does not bleed into
// tree/fence silhouettes when the texture switches to a smaller mip. Where all alpha is 0
// the plain average is kept: on opaque materials the alpha channel is often only the envmap
// mask, and 0 there (lamps, wheels) must not turn the colour black.
//
// `fs_plain` averages colour and alpha each on its own, as OMSI does for text and script
// textures (the cab displays, `[CTC]` pictures): alpha-weighted, the white of a digit kept
// full brightness out to the edge of its fading alpha, and digits grew bold and ran together
// from a few metres off.
@group(0) @binding(0) var t_src: texture_2d<f32>;
@group(0) @binding(1) var s_src: sampler;
struct VsOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    var out: VsOut;
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

// The k-th source texel under destination texel `i` along one axis (source size `s`,
// destination size `d`) and its weight. Counted in 1/d of a source texel the destination
// texel spans [i*s, (i+1)*s) and source texel j spans [j*d, (j+1)*d): all whole numbers, so
// the weights are exact and add up to 1. A footprint touches at most 4 texels (s <= 2d+1).
fn tap(i: u32, s: u32, d: u32, k: u32) -> vec2<f32> {
    let lo = i * s;
    let hi = lo + s;
    let j = lo / d + k;
    let a = max(j * d, lo);
    let b = min(j * d + d, hi);
    let w = select(0.0, f32(b - a) / f32(s), b > a);
    return vec2<f32>(f32(min(j, s - 1u)), w);
}

struct Box { weighted: vec3<f32>, plain: vec3<f32>, alpha: f32 };

fn box_average(pos: vec2<f32>) -> Box {
    let src = textureDimensions(t_src);
    let dst = max(src / 2u, vec2<u32>(1u));
    let at = min(vec2<u32>(pos), dst - vec2<u32>(1u));
    var b: Box;
    b.weighted = vec3<f32>(0.0);
    b.plain = vec3<f32>(0.0);
    b.alpha = 0.0;
    for (var ky = 0u; ky < 4u; ky = ky + 1u) {
        let ty = tap(at.y, src.y, dst.y, ky);
        if (ty.y <= 0.0) {
            continue;
        }
        for (var kx = 0u; kx < 4u; kx = kx + 1u) {
            let tx = tap(at.x, src.x, dst.x, kx);
            let w = tx.y * ty.y;
            if (w <= 0.0) {
                continue;
            }
            let q = textureLoad(t_src, vec2<i32>(i32(tx.x), i32(ty.x)), 0);
            b.weighted = b.weighted + q.rgb * (q.a * w);
            b.plain = b.plain + q.rgb * w;
            b.alpha = b.alpha + q.a * w;
        }
    }
    return b;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let b = box_average(in.clip.xy);
    return vec4<f32>(select(b.plain, b.weighted / b.alpha, b.alpha > 2.5e-6), b.alpha);
}

@fragment
fn fs_plain(in: VsOut) -> @location(0) vec4<f32> {
    let b = box_average(in.clip.xy);
    return vec4<f32>(b.plain, b.alpha);
}
