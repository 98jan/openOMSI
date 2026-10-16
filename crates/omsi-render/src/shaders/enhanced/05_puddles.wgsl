// Small waves on the water at map point `p` (m) and time `t` (s): the slope of four wave
// trains of 1.3 to 4.7 m running different ways.
fn water_ripple(p: vec2<f32>, t: f32) -> vec2<f32> {
    var g = vec2<f32>(0.0);
    // six trains of waves whose lengths share no common measure, so that the pattern does
    // not repeat across a lake (#1822: four of them tiled a large water every few metres)
    let dirs = array<vec2<f32>, 6>(vec2<f32>(0.8, 0.6), vec2<f32>(-0.45, 0.89), vec2<f32>(0.96, -0.28), vec2<f32>(-0.7, -0.71), vec2<f32>(0.34, 0.94), vec2<f32>(-0.99, 0.14));
    let lens = array<f32, 6>(6.7, 4.7, 2.9, 1.9, 1.3, 0.83);
    // the wind rakes the water in patches (its gusts): the slopes come and go across it
    let gusts = 0.55 + 0.9 * vnoise_f(p, 0.045, vec2<f32>(3.1, -7.7));
    for (var i = 0; i < 6; i = i + 1) {
        let k = 6.2831853 / lens[i];
        // deep-water waves: their speed goes with the square root of their length
        let w = sqrt(9.81 * k);
        g = g + dirs[i] * cos(dot(p, dirs[i]) * k - w * t) * 0.037;
    }
    return g * gusts;
}

// Shade the complete local vehicle from the reflected eye. Main-camera ambient
// occlusion is not applicable to this view beneath the road.
@fragment
fn fs_puddle_vehicle(input: FsIn) -> @location(0) vec4<f32> {
    // One reflected camera for the complete vehicle, including its transparent panes.
    let plane = vehicle_reflection.plane;
    let height = dot(plane.xyz, input.world) - plane.w;
    if (height < 0.0 || material.emissive.w > 1.5) { discard; }
    var unused = vec2<f32>(0.0);
    let eye = camera.cam_pos.xyz - 2.0 * plane.xyz * (dot(plane.xyz, camera.cam_pos.xyz) - plane.w);
    if (camera.post.x < 0.5) {
        var unused_vanilla = 0.0;
        return shade_vanilla(input, &unused_vanilla, eye);
    }
    return shade_enhanced(input, &unused, true, eye);
}

@fragment
fn fs_puddle_chassis() -> @location(0) vec4<f32> {
    if (camera.post.x < 0.5) { return vec4<f32>(camera.ambient.rgb * 0.025, 1.0); }
    return vec4<f32>(sh_irradiance(vec3<f32>(0.0, 0.0, -1.0)) * enh.exposure.x * 0.025, 1.0);
}
