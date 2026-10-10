use super::*;

/// The night meter takes the street the picture shows: from 30 m over a lot lit by a
/// grid of street lamps, looking down at it, the eye adapts to the lit lot - not, as
/// when it sampled a "ground" 1.6 m under the camera (in the air, out of every lamp's
/// reach), to a moonless night that lifted the lot's light to white.
#[test]
fn the_night_meter_sees_a_lamp_lit_lot_from_above() {
    let mut lights = Vec::new();
    for i in 0..4 {
        for j in 0..4 {
            lights.push(PointLight {
                position: DVec3::new(i as f64 * 12.0 - 18.0, j as f64 * 12.0 - 18.0, 8.0),
                radius: 24.0,
                color: [1.0, 0.8, 0.5],
                intensity: 1.0,
                core: 3.0,
                housed: true,
                mode: LightMode::Both,
                ..Default::default()
            });
        }
    }
    let cam = Camera { position: DVec3::new(0.0, -35.0, 30.0), yaw: 0.0, pitch: -40.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 1000.0 };
    let cam_rel = cam.position.as_vec3();
    let seen = view_lamp_light(&lights, DVec3::ZERO, cam_rel, &cam, 16.0 / 9.0, Some(0.0));
    let in_air = view_lamp_light(&lights, DVec3::ZERO, cam_rel, &cam, 16.0 / 9.0, None);
    // (the lot's ground: some 10-30 lux under the lamps, 1e-3 .. 3e-3, less between and
    // round them - a log-average of some 10 lux; a moonless night is 2e-6)
    assert!(seen > 5e-5, "{seen}");
    assert!(in_air < seen * 0.2, "{in_air} vs {seen}");
    // and the lamps seen from beside the lot, at eye height, veil the eye as well
    let eye = Camera { position: DVec3::new(0.0, -40.0, 1.7), pitch: 0.0, ..cam };
    assert!(glare_veil(&lights, DVec3::ZERO, eye.position.as_vec3(), eye.forward()) > 1e-5);
}

/// A textured material can have black diffuse but white ambient (depot interiors).
/// Enhanced must not turn it into a black surface or silently replace its diffuse.
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn enhanced_respects_material_ambient() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    let mesh = renderer.add_mesh(&mut scene, &MeshData {
        positions: vec![Vec3::new(-8.0, 4.0, -8.0), Vec3::new(8.0, 4.0, -8.0), Vec3::new(8.0, 4.0, 8.0), Vec3::new(-8.0, 4.0, 8.0)],
        normals: vec![-Vec3::Y; 4],
        uvs: vec![glam::Vec2::ZERO; 4],
        ranges: vec![(0, 6, 0)],
        indices: vec![0, 1, 2, 0, 2, 3],
        one_sided: false,
    });
    let texture = renderer.add_texture(&mut scene, &omsi_texture::Image {
        width: 1, height: 1, rgba: vec![180, 180, 180, 255], has_alpha: true,
    }, false);
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 90.0, near: 0.1, far: 100.0 };
    let lighting = Lighting { enhanced: true, shadows: false, fog_density: 0.0, sun_dir: -Vec3::Y, ..Default::default() };
    let mut pixels = Vec::new();
    for ambient in [[0.0; 3], [1.0; 3], [1.0, 0.0, 0.0]] {
        let material = renderer.add_material_extra(&mut scene, Some(texture), AlphaMode::Blend,
            [0.0, 0.0, 0.0, 1.0], false, None, None, None, None, [0.0; 3],
            MaterialExtra { ambient: Some(ambient), ..Default::default() });
        let id = renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![material]);
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        pixels.push(rgba[(32 * 64 + 32) * 4..][..3].to_vec());
        scene.instances[id].visible = false;
    }
    assert!(pixels[1].iter().map(|&v| v as u32).sum::<u32>() > pixels[0].iter().map(|&v| v as u32).sum::<u32>() + 60,
        "white ambient must illuminate black diffuse: {pixels:?}");
    assert!(pixels[2][0] > pixels[2][1].saturating_add(20) && pixels[2][0] > pixels[2][2].saturating_add(20),
        "the material's ambient tint must be preserved: {pixels:?}");
}

/// A road must cover flush terrain without covering a building floor above it.
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn surface_bias_keeps_raised_floors_visible_at_a_distance() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    let mesh = renderer.add_mesh(&mut scene, &MeshData {
        positions: vec![Vec3::new(-1000.0, -1000.0, 0.0), Vec3::new(1000.0, -1000.0, 0.0), Vec3::new(1000.0, 1000.0, 0.0), Vec3::new(-1000.0, 1000.0, 0.0)],
        normals: vec![Vec3::Z; 4], uvs: vec![glam::Vec2::ZERO; 4],
        ranges: vec![(0, 6, 0)], indices: vec![0, 1, 2, 0, 2, 3], one_sided: false,
    });
    let blue = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.0, 0.0, 1.0, 1.0], true);
    let green = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.0, 1.0, 0.0, 1.0], true);
    let red = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0, 0.0, 0.0, 1.0], true);
    let ground = renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![blue]);
    scene.instances[ground].render_phase = RenderPhase::Terrain;
    let road = renderer.add_surface_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![green]);
    scene.instances[road].render_phase = RenderPhase::Spline;
    let floor = renderer.add_instance(&mut scene, mesh, DVec3::new(0.0, 0.0, 0.03), Mat4::IDENTITY, vec![red]);
    scene.instances[floor].render_phase = RenderPhase::AfterVehicles;
    let lighting = Lighting { shadows: false, fog_density: 0.0, ..Default::default() };
    for (pitch, fov) in [(-5.0, 60.0), (-5.0, 20.0), (-30.0, 60.0)] {
        let camera = Camera { position: DVec3::new(0.0, 0.0, 5.0), yaw: 0.0, pitch, roll: 0.0, fov_deg: fov, near: 0.1, far: 2000.0 };
        for show_floor in [false, true] {
            scene.instances[floor].visible = show_floor;
            scene.dirty = true;
            let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
            let pixel = &rgba[(32 * 64 + 32) * 4..][..3];
            let wanted = if show_floor { 0 } else { 1 };
            assert!(pixel[wanted] > 200 && pixel[1 - wanted] < 20 && pixel[2] < 20,
                "pitch {pitch}, fov {fov}, floor {show_floor}: {pixel:?}");
        }
    }
}

/// An untextured slot (a Blender export's Base Color alone) keeps its colour in Enhanced:
/// the white ambient Omsi.exe gives every o3d slot turned it white under the sky (#1737).
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn enhanced_untextured_keeps_its_colour() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    let mesh = renderer.add_mesh(&mut scene, &MeshData {
        positions: vec![Vec3::new(-8.0, 4.0, -8.0), Vec3::new(8.0, 4.0, -8.0), Vec3::new(8.0, 4.0, 8.0), Vec3::new(-8.0, 4.0, 8.0)],
        normals: vec![-Vec3::Y; 4],
        uvs: vec![glam::Vec2::ZERO; 4],
        ranges: vec![(0, 6, 0)],
        indices: vec![0, 1, 2, 0, 2, 3],
        one_sided: false,
    });
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 90.0, near: 0.1, far: 100.0 };
    let lighting = Lighting { enhanced: true, shadows: false, fog_density: 0.0, sun_dir: -Vec3::Y, ..Default::default() };
    let material = renderer.add_material_extra(&mut scene, None, AlphaMode::Opaque,
        [0.8, 0.05, 0.05, 1.0], false, None, None, None, None, [0.0; 3],
        MaterialExtra { ambient: Some([1.0; 3]), ..Default::default() });
    renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![material]);
    let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
    let px = &rgba[(32 * 64 + 32) * 4..][..3];
    assert!(px[0] as u32 > px[1] as u32 * 2 + 10 && px[0] as u32 > px[2] as u32 * 2 + 10, "an untextured red slot must stay red: {px:?}");
}

/// Overlays drawn texel for pixel: onto whole pixels, their size kept.
#[test]
fn overlays_land_on_whole_pixels() {
    // (a 120 x 26 text a quarter and a half pixel off: moved, the same size)
    assert_eq!(snap_rect([25.25, 40.5, 145.25, 66.5]), [25.0, 41.0, 145.0, 67.0]);
    assert_eq!(snap_rect([10.0, 20.0, 30.0, 40.0]), [10.0, 20.0, 30.0, 40.0]);
    // (left of the window as well: the same size)
    assert_eq!(snap_rect([-0.5, -2.5, 19.5, 7.5]), [0.0, -2.0, 20.0, 8.0]);
    // (a separator 0.6 px high stays a line)
    let line = snap_rect([16.0, 100.3, 300.0, 100.9]);
    assert_eq!(line[3] - line[1], 1.0);
    // (an empty rectangle stays empty)
    assert_eq!(snap_rect([5.2, 5.2, 5.2, 5.2]), [5.0, 5.0, 5.0, 5.0]);
}

/// The enhanced street is lit by the sun that gets through the weather: none from
/// under an overcast deck, next to none through a dense fog (which glowed white round
/// the sun), nearly all of it on a clear or a hazy summer's day (#1106).
#[test]
fn enhanced_sun_does_not_shine_through_overcast_or_dense_fog() {
    // an evening sun 27 degrees high, a weather's visibility as the app turns it into
    // a density (`lights::lighting_from`), the cover and the sun `apply_weather` leaves
    let light = |visibility: f32, cloud_density: f32, sun_intensity: f32| Lighting {
        sun_dir: Vec3::new(-0.89, 0.0, 0.454),
        fog_density: (2.3 / visibility).max(0.00005),
        cloud_density,
        sun_intensity,
        ..Default::default()
    };
    let reaching = |l: &Lighting| enhanced_sky_input(l, 1.0).0.sun_visibility;
    // #CAVOK, a cumulus sky, Sommerlich's summer haze of 2 km
    assert!(reaching(&light(50_000.0, 0.0, 1.0)) > 0.99);
    assert!(reaching(&light(50_000.0, 0.75, 0.54)) > 0.53);
    assert!(reaching(&light(2_000.0, 0.35, 1.0)) > 0.4);
    // Bodennebel's 75 m, Schmuddelwetter's 200 m: the sun is gone
    assert!(reaching(&light(75.0, 0.0, 1.0)) < 1e-4);
    assert!(reaching(&light(200.0, 0.0, 1.0)) < 1e-3);
    // an Overcast cloud type (the 15 % of the sun `apply_weather` keeps): gone from the
    // street, while the clouds keep their light from above
    let (input, clouds) = enhanced_sky_input(&light(50_000.0, 1.0, 0.15), 1.0);
    assert_eq!(input.sun_visibility, 0.0);
    assert!((clouds - 0.15).abs() < 1e-6);
}

#[test]
#[ignore = "requires a graphics adapter; renders terrain and foliage lighting"]
fn enhanced_masked_and_uncut_ground_share_lighting() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            ssao: false,
            shadow_size: 1024,
            fxaa: false,
            render_scale: 1.0,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let mut texture = |rgba: [u8; 4]| {
        renderer.add_texture(
            &mut scene,
            &omsi_texture::Image {
                width: 1,
                height: 1,
                rgba: rgba.to_vec(),
                has_alpha: true,
            },
            false,
        )
    };
    let grey = texture([100, 100, 100, 255]);
    let opaque = texture([255; 4]);
    let transparent = texture([100, 100, 100, 0]);
    let masked = renderer.add_terrain_material(
        &mut scene,
        Some(grey),
        Some(opaque),
        None,
        1.0,
        None,
        0.0,
    );
    let uncut =
        renderer.add_terrain_material(&mut scene, Some(grey), None, None, 1.0, None, 0.0);
    let cut = renderer.add_terrain_material(
        &mut scene,
        Some(grey),
        Some(transparent),
        None,
        1.0,
        None,
        0.0,
    );
    let foliage =
        renderer.add_material(&mut scene, Some(grey), AlphaMode::Test, [1.0; 4], false);
    let cut_foliage = renderer.add_material(
        &mut scene,
        Some(transparent),
        AlphaMode::Test,
        [1.0; 4],
        false,
    );
    let backdrop = renderer.add_material(
        &mut scene,
        None,
        AlphaMode::Opaque,
        [1.0, 0.0, 0.0, 1.0],
        true,
    );
    let mut quad = |left: f32, right: f32, z: f32, material| {
        let mesh = renderer.add_mesh(
            &mut scene,
            &MeshData {
                positions: vec![
                    Vec3::new(left, -5.0, z),
                    Vec3::new(right, -5.0, z),
                    Vec3::new(right, 5.0, z),
                    Vec3::new(left, 5.0, z),
                ],
                normals: vec![Vec3::Z; 4],
                uvs: vec![glam::Vec2::splat(0.5); 4],
                indices: vec![0, 1, 2, 0, 2, 3],
                ranges: vec![(0, 6, 0)],
                one_sided: false,
            },
        );
        renderer.add_instance(
            &mut scene,
            mesh,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![material],
        )
    };
    // Symmetric samples in the same image share exposure, view and lamp distances.
    quad(-6.0, 6.0, -1.0, backdrop);
    let ground = quad(-5.0, -0.5, 0.0, masked);
    let mapped = quad(0.5, 5.0, 0.0, uncut);
    scene.instances[ground].render_phase = RenderPhase::Terrain;
    scene.instances[mapped].render_phase = RenderPhase::Spline;
    scene.instances[mapped].surface = true;
    let camera = Camera {
        position: DVec3::new(0.0, -0.105, 6.0),
        yaw: 0.0,
        pitch: -89.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    let day = Lighting {
        enhanced: true,
        sun_dir: Vec3::Z,
        sun_intensity: 1.0,
        shadows: false,
        detail: false,
        fog_density: 0.0,
        ..Default::default()
    };
    let night = Lighting {
        sun_dir: -Vec3::Z,
        sun_intensity: 0.0,
        night: 1.0,
        ..day.clone()
    };
    let pixel = |rgba: &[u8], x: usize| -> [u8; 3] {
        rgba[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 3]
            .try_into()
            .unwrap()
    };
    for (name, lighting) in [("sun", &day), ("lamp", &night)] {
        scene.lights = if name == "lamp" {
            vec![PointLight {
                position: DVec3::new(0.0, 0.0, 4.0),
                radius: 20.0,
                core: 10.0,
                intensity: 1.0,
                ..Default::default()
            }]
        } else {
            Vec::new()
        };
        let rgba = renderer
            .render_to_image(&mut scene, 64, 64, &camera, lighting)
            .unwrap();
        let (a, b) = (pixel(&rgba, 16), pixel(&rgba, 47));
        assert!(
            a.iter().all(|v| *v > 10 && *v < 245),
            "lit, unclipped {name}: {a:?}"
        );
        assert!(
            a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 2),
            "masked terrain and uncut mapped ground differ under {name}: {a:?} / {b:?}"
        );
    }
    // A road cut still reveals the geometry beneath it.
    renderer.set_material(&mut scene, ground, 0, cut);
    let rgba = renderer
        .render_to_image(&mut scene, 64, 64, &camera, &night)
        .unwrap();
    let a = pixel(&rgba, 16);
    assert!(
        a[0] > a[1] + 30 && a[0] > a[2] + 30,
        "road cut must reveal red: {a:?}"
    );

    // Real foliage still scatters light arriving from behind its normal; ground does not.
    renderer.set_material(&mut scene, ground, 0, masked);
    renderer.set_material(&mut scene, mapped, 0, foliage);
    scene.lights[0].position.z = -4.0;
    let rgba = renderer
        .render_to_image(&mut scene, 64, 64, &camera, &night)
        .unwrap();
    let (a, b) = (pixel(&rgba, 16), pixel(&rgba, 47));
    assert!(
        b[1] > a[1] + 8,
        "foliage retains backlighting: ground {a:?}, foliage {b:?}"
    );
    renderer.set_material(&mut scene, mapped, 0, cut_foliage);
    let rgba = renderer
        .render_to_image(&mut scene, 64, 64, &camera, &night)
        .unwrap();
    let b = pixel(&rgba, 47);
    assert!(
        b[0] > b[1] + 30 && b[0] > b[2] + 30,
        "foliage cutout must reveal red: {b:?}"
    );
}

#[test]
fn a_mesh_page_hands_freed_space_to_the_next_mesh_and_joins_its_holes() {
    let (mut free, mut top) = (Vec::new(), 0u64);
    let a = page_take(&mut free, &mut top, 1000, 100).unwrap();
    let b = page_take(&mut free, &mut top, 1000, 200).unwrap();
    let c = page_take(&mut free, &mut top, 1000, 300).unwrap();
    assert_eq!((a, b, c, top), (0, 100, 300, 600));
    page_give(&mut free, &mut top, b, 200);
    assert_eq!(free, vec![(100, 200)]);
    assert_eq!(page_take(&mut free, &mut top, 1000, 150), Some(100));
    assert_eq!(free, vec![(250, 50)]);
    page_give(&mut free, &mut top, a, 100);
    page_give(&mut free, &mut top, 100, 150);
    assert_eq!(free, vec![(0, 300)]);
    page_give(&mut free, &mut top, c, 300);
    assert_eq!((free.len(), top), (0, 0));
    assert_eq!(page_take(&mut free, &mut top, 1000, 1001), None);
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn meshes_coming_and_going_tile_by_tile_keep_the_pages_from_growing() {
    let adapter = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(Renderer::new_with(
        &adapter, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    )).expect("test renderer");
    if !renderer.mesh_pages() {
        return;
    }
    let mut scene = renderer.new_scene();
    let mesh = |n: usize| {
        let positions: Vec<Vec3> = (0..n).map(|i| Vec3::new(i as f32, (i % 7) as f32, 0.0)).collect();
        let indices: Vec<u32> = (0..n as u32 / 3 * 3).collect();
        MeshData { normals: vec![Vec3::Z; n], uvs: vec![glam::Vec2::ZERO; n], ranges: vec![(0, indices.len() as u32, 0)], positions, indices, ..Default::default() }
    };
    // tiles of a few thousand meshes of every size, loaded and unloaded in no order, as
    // a long drive across a map brings them; the scene's pages must not keep growing
    let mut tiles: Vec<Vec<MeshId>> = Vec::new();
    let mut seed = 0x1234_5678u64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut most = 0;
    for round in 0..300 {
        let tile: Vec<MeshId> = (0..400).map(|_| renderer.add_mesh(&mut scene, &mesh(3 + (rand() % 600) as usize))).collect();
        tiles.push(tile);
        if tiles.len() > 12 {
            let gone = tiles.remove((rand() % tiles.len() as u64) as usize);
            for id in gone {
                renderer.free_mesh(&mut scene, id);
            }
        }
        if round == 50 {
            most = scene.mesh_pages.len();
        }
    }
    assert!(scene.mesh_pages.len() <= most + 1, "the pages grew from {most} to {} over the drive", scene.mesh_pages.len());
    let held = scene.mesh_page_bytes();
    let used: u64 = scene.meshes.iter().filter(|m| m.page != u32::MAX).map(|m| m.vertex_bytes + m.index_bytes).sum();
    assert!(held < used * 3, "{held} bytes of pages held for {used} in use");
    for t in &tiles {
        for &id in t {
            let m = &scene.meshes[id];
            assert_eq!(m.vertex_offset % std::mem::size_of::<Vertex>() as u64, 0);
            assert!(m.vertex_offset + m.vertex_bytes <= m.vertex_buf.size());
            assert!(m.first_index as u64 * 4 + m.index_bytes <= m.index_buf.size());
        }
    }
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn msaa_depth_prepass_preserves_layered_surfaces() {
    fn quad(renderer: &Renderer, scene: &mut Scene, z: f32, mat: MaterialId) -> usize {
        let mesh = renderer.add_mesh(
            scene,
            &MeshData {
                positions: vec![
                    Vec3::new(-4.0, -4.0, z),
                    Vec3::new(4.0, -4.0, z),
                    Vec3::new(4.0, 4.0, z),
                    Vec3::new(-4.0, 4.0, z),
                ],
                normals: vec![Vec3::Z; 4],
                uvs: vec![
                    glam::Vec2::ZERO,
                    glam::Vec2::X,
                    glam::Vec2::ONE,
                    glam::Vec2::Y,
                ],
                indices: vec![0, 1, 2, 0, 2, 3],
                ranges: vec![(0, 6, 0)],
                one_sided: false,
            },
        );
        renderer.add_surface_instance(scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![mat])
    }
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 4,
            ssao: true,
            fxaa: false,
            render_scale: 1.0,
            ..Default::default()
        },
    ))
    .expect("test renderer");
    if renderer.options.msaa <= 1 {
        return; // This adapter cannot exercise a multisampled depth buffer.
    }
    assert!(renderer.prepass_msaa_pipelines.is_some());
    let mut scene = renderer.new_scene();
    let mask = renderer.add_texture(
        &mut scene,
        &omsi_texture::Image {
            width: 7,
            height: 1,
            rgba: [0, 0, 112, 128, 160, 255, 255]
                .into_iter()
                .flat_map(|a| [255, 255, 255, a])
                .collect(),
            has_alpha: true,
        },
        false,
    );
    let base = renderer.add_terrain_material(&mut scene, None, None, None, 1.0, None, 1.0);
    let ground = quad(&renderer, &mut scene, 0.0, base);
    scene.instances[ground].render_phase = RenderPhase::Terrain;
    let paint =
        renderer.add_terrain_layer_material(&mut scene, None, mask, None, 1.0, None, 1.0);
    let layer = quad(&renderer, &mut scene, 0.0, paint);
    scene.instances[layer].ground_layer = true;
    scene.instances[layer].render_phase = RenderPhase::Terrain;
    let road_mat = renderer.add_material(
        &mut scene,
        None,
        AlphaMode::Opaque,
        [0.0, 0.0, 1.0, 1.0],
        true,
    );
    let road = quad(&renderer, &mut scene, 0.02, road_mat);
    scene.instances[road].render_phase = RenderPhase::Spline;
    let cutout = renderer.add_material(
        &mut scene,
        Some(mask),
        AlphaMode::Test,
        [0.0, 1.0, 0.0, 1.0],
        false,
    );
    let fence = quad(&renderer, &mut scene, 0.3, cutout);
    scene.instances[fence].render_phase = RenderPhase::Normal;
    let blend = renderer.add_material(
        &mut scene,
        None,
        AlphaMode::Blend,
        [1.0, 0.0, 0.0, 0.25],
        false,
    );
    let pane = quad(&renderer, &mut scene, 0.5, blend);
    scene.instances[pane].render_phase = RenderPhase::Normal;
    // A small excavation in the corner activates the deferred prepass without
    // covering the road, cutout and glass samples checked below.
    let floor = quad(&renderer, &mut scene, -0.1, road_mat);
    scene.instances[floor].presurface = true;
    scene.instances[floor].transform = Mat4::from_translation(Vec3::new(7.0, 7.0, 0.0));
    let clear = renderer.add_texture(&mut scene, &omsi_texture::Image {
        width: 1, height: 1, rgba: vec![255, 255, 255, 0], has_alpha: true,
    }, false);
    let cover_material = renderer.add_material(&mut scene, Some(clear), AlphaMode::Blend, [1.0; 4], true);
    let cover = quad(&renderer, &mut scene, 0.1, cover_material);
    scene.instances[cover].presurface = true;
    scene.instances[cover].transform = Mat4::from_translation(Vec3::new(7.0, 7.0, 0.0));
    let camera = Camera {
        position: DVec3::new(0.0, -0.105, 6.0),
        yaw: 0.0,
        pitch: -89.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    for (excavation, wetness) in [(false, 0.0), (false, 1.0), (true, 0.0), (true, 1.0)] {
        scene.instances[floor].visible = excavation;
        scene.instances[cover].visible = excavation;
        Renderer::mark_changed(&mut scene, floor);
        Renderer::mark_changed(&mut scene, cover);
        let lighting = Lighting {
            enhanced: true,
            shadows: false,
            fog_density: 0.0,
            wetness,
            ..Default::default()
        };
        let enabled = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        let pipelines = renderer.prepass_msaa_pipelines.take().unwrap();
        let disabled = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        renderer.prepass_msaa_pipelines = Some(pipelines);
        let delta = enabled
            .iter()
            .zip(&disabled)
            .map(|(a, b)| a.abs_diff(*b))
            .max()
            .unwrap();
        assert!(
            delta <= 2,
            "MSAA prepass changed layered surfaces by {delta}: excavation {excavation}, wetness {wetness}"
        );
        let pixel = |x: usize| &enabled[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 3];
        assert!(
            pixel(12)[2] > pixel(12)[1] + 30,
            "cutout hid the road: {:?}",
            pixel(12)
        );
        assert!(
            pixel(52)[1] > pixel(52)[2] + 30,
            "opaque cutout disappeared: {:?}",
            pixel(52)
        );
    }
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn a_reshaped_mesh_moves_the_bounds_of_every_instance_drawing_it() {
    let adapter = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(Renderer::new_with(
        &adapter, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    scene.cache_bounds = true;
    let material = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0; 4], true);
    let data = MeshData {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        normals: vec![Vec3::Z; 3], uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2], ranges: vec![(0, 3, 0)], ..Default::default()
    };
    let (skinned, plain) = (renderer.add_mesh(&mut scene, &data), renderer.add_mesh(&mut scene, &data));
    let add = |scene: &mut Scene, mesh, x: f64| renderer.add_instance(scene, mesh, DVec3::new(x, 0.0, 0.0), Mat4::IDENTITY, vec![material]);
    let first = add(&mut scene, skinned, 0.0);
    let others: Vec<usize> = (0..300).map(|k| add(&mut scene, plain, k as f64 * 10.0)).collect();
    renderer.prepare(&mut scene);
    let pose = |scale: f32| -> Vec<Vec3> { data.positions.iter().map(|p| *p * scale).collect() };
    renderer.update_mesh(&mut scene, skinned, &pose(5.0), &data.normals, &data.uvs);
    renderer.prepare(&mut scene);
    // one added after the first pose, one switched to the skinned mesh
    let late = add(&mut scene, skinned, 50.0);
    renderer.set_instance_mesh(&mut scene, others[7], skinned);
    renderer.prepare(&mut scene);
    renderer.update_mesh(&mut scene, skinned, &pose(9.0), &data.normals, &data.uvs);
    renderer.prepare(&mut scene);
    let expected = InstanceBounds::new(&scene.meshes[skinned], Mat4::IDENTITY);
    for i in [first, late, others[7]] {
        assert_eq!(scene.instances[i].bounds.radius, expected.radius, "instance {i}");
    }
    assert_ne!(scene.instances[others[8]].bounds.radius, expected.radius);
    assert!(scene.bounds_users.len() <= 4, "{} instances rescanned", scene.bounds_users.len());
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn terrain_paint_fast_path_preserves_composition() {
    fn quad(
        renderer: &Renderer,
        scene: &mut Scene,
        rect: [f32; 4],
        z: f32,
        mat: MaterialId,
    ) -> usize {
        let [left, right, bottom, top] = rect;
        let mesh = renderer.add_mesh(
            scene,
            &MeshData {
                positions: vec![
                    Vec3::new(left, bottom, z),
                    Vec3::new(right, bottom, z),
                    Vec3::new(right, top, z),
                    Vec3::new(left, top, z),
                ],
                normals: vec![Vec3::Z; 4],
                uvs: vec![
                    glam::Vec2::ZERO,
                    glam::Vec2::X,
                    glam::Vec2::ONE,
                    glam::Vec2::Y,
                ],
                indices: vec![0, 1, 2, 0, 2, 3],
                ranges: vec![(0, 6, 0)],
                one_sided: false,
            },
        );
        renderer.add_surface_instance(scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![mat])
    }
    for msaa in [1, 4] {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let mut renderer = pollster::block_on(Renderer::new_with(
            &instance,
            None,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            RenderOptions {
                msaa,
                ssao: false,
                shadow_size: 1024,
                fxaa: false,
                render_scale: 1.0,
                ..Default::default()
            },
        ))
        .expect("test renderer");
        let mut scene = renderer.new_scene();
        let base = renderer.add_terrain_material(&mut scene, None, None, None, 1.0, None, 1.0);
        let ground = quad(&renderer, &mut scene, [-6.0, 6.0, -6.0, 6.0], 0.0, base);
        scene.instances[ground].render_phase = RenderPhase::Terrain;
        let mut paint_materials = Vec::new();
        for (rgb, alphas) in [
            ([160, 40, 40], [0, 0, 112, 128, 160, 255, 255]),
            ([40, 160, 40], [255, 255, 160, 128, 112, 0, 0]),
        ] {
            let diffuse = renderer.add_texture(
                &mut scene,
                &omsi_texture::Image {
                    width: 4,
                    height: 4,
                    rgba: (0..16)
                        .flat_map(|i| {
                            let c = rgb.map(|v| {
                                if (i / 4 + i % 4) % 2 == 0 { v } else { v / 2 }
                            });
                            [c[0], c[1], c[2], 255]
                        })
                        .collect(),
                    has_alpha: false,
                },
                false,
            );
            let mask = renderer.add_texture(
                &mut scene,
                &omsi_texture::Image {
                    width: 7,
                    height: 1,
                    rgba: alphas
                        .into_iter()
                        .flat_map(|a| [255, 255, 255, a])
                        .collect(),
                    has_alpha: true,
                },
                false,
            );
            // Generated tiled detail exercises sampling beside empty mask pixels.
            let detail = renderer.add_texture(
                &mut scene,
                &omsi_texture::Image {
                    width: 4,
                    height: 4,
                    rgba: (0..16)
                        .flat_map(|i| {
                            let v = if (i / 4 + i % 4) % 2 == 0 { 160 } else { 255 };
                            [v, v, v, 255]
                        })
                        .collect(),
                    has_alpha: false,
                },
                false,
            );
            let mat = renderer.add_terrain_layer_material(
                &mut scene,
                Some(diffuse),
                mask,
                Some((detail, 12.0)),
                8.0,
                None,
                1.0,
            );
            paint_materials.push(mat);
            let layer = quad(&renderer, &mut scene, [-6.0, 6.0, -6.0, 6.0], 0.0, mat);
            scene.instances[layer].ground_layer = true;
            scene.instances[layer].render_phase = RenderPhase::Terrain;
        }
        let blue = renderer.add_material(
            &mut scene,
            None,
            AlphaMode::Opaque,
            [0.0, 0.0, 1.0, 1.0],
            true,
        );
        let road = quad(&renderer, &mut scene, [-6.0, 6.0, -0.7, 0.7], 0.02, blue);
        scene.instances[road].render_phase = RenderPhase::Spline;
        let camera = Camera {
            position: DVec3::new(0.0, -0.105, 6.0),
            yaw: 0.0,
            pitch: -89.0,
            roll: 0.0,
            fov_deg: 90.0,
            near: 0.1,
            far: 100.0,
        };
        // Compare identical scene data through the specialized and original no-write
        // pipelines, including water's opacity and wet-ground reflection coverage.
        for (wetness, water) in [(0.0, false), (1.0, false), (1.0, true)] {
            if water {
                for &mat in &paint_materials {
                    scene.materials[mat].uniform.ambient[3] = 2.0;
                    renderer.queue.write_buffer(
                        &scene.materials[mat].buf,
                        0,
                        bytemuck::bytes_of(&scene.materials[mat].uniform),
                    );
                }
            }
            let lighting = Lighting {
                enhanced: true,
                shadows: false,
                fog_density: 0.0,
                wetness,
                ..Default::default()
            };
            let actual = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &lighting)
                .unwrap();
            let pp = renderer.hdr_pass.as_mut().unwrap();
            let mut saved = Vec::new();
            for variant in 0..4 {
                let original = pp.pipelines[PIPE_BLEND_NO_WRITE as usize * 4 + variant].clone();
                saved.push(std::mem::replace(
                    &mut pp.pipelines[PIPE_TERRAIN_PAINT as usize * 4 + variant],
                    original,
                ));
            }
            let expected = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &lighting)
                .unwrap();
            for (variant, pipeline) in saved.into_iter().enumerate() {
                renderer.hdr_pass.as_mut().unwrap().pipelines
                    [PIPE_TERRAIN_PAINT as usize * 4 + variant] = pipeline;
            }
            let delta = actual
                .iter()
                .zip(&expected)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(
                delta <= 2,
                "paint composition changed by {delta}: MSAA {msaa}, wet {wetness}, water {water}"
            );
            let centre = &actual[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
            if debug_view() <= 0.5 {
                assert!(
                    centre[2] > centre[0] + 30 && centre[2] > centre[1] + 30,
                    "paint hid the road: {centre:?}"
                );
            }
        }
    }
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn a_freed_and_recycled_mesh_slot_is_no_longer_counted_as_reshaped() {
    let adapter = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(Renderer::new_with(
        &adapter, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    scene.cache_bounds = true;
    let material = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0; 4], true);
    let data = MeshData {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        normals: vec![Vec3::Z; 3], uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2], ranges: vec![(0, 3, 0)], ..Default::default()
    };
    let pose = |scale: f32| -> Vec<Vec3> { data.positions.iter().map(|p| *p * scale).collect() };
    let (once_skinned, skinned) = (renderer.add_mesh(&mut scene, &data), renderer.add_mesh(&mut scene, &data));
    let first = renderer.add_instance(&mut scene, skinned, DVec3::ZERO, Mat4::IDENTITY, vec![material]);
    renderer.update_mesh(&mut scene, once_skinned, &pose(2.0), &data.normals, &data.uvs);
    renderer.prepare(&mut scene);
    // the slot is freed and taken by a plain mesh, which a whole tile then draws
    renderer.free_mesh(&mut scene, once_skinned);
    renderer.prepare(&mut scene);
    let plain = renderer.add_mesh(&mut scene, &data);
    assert_eq!(renderer.recycle_mesh(&mut scene, plain, once_skinned), once_skinned);
    let tile: Vec<usize> = (0..200).map(|k| renderer.add_instance(&mut scene, once_skinned, DVec3::new(k as f64, 0.0, 0.0), Mat4::IDENTITY, vec![material])).collect();
    renderer.prepare(&mut scene);
    renderer.update_mesh(&mut scene, skinned, &pose(7.0), &data.normals, &data.uvs);
    renderer.prepare(&mut scene);
    assert!(scene.bounds_users.len() <= 2, "{} instances rescanned", scene.bounds_users.len());
    let expected = InstanceBounds::new(&scene.meshes[skinned], Mat4::IDENTITY);
    assert_eq!(scene.instances[first].bounds.radius, expected.radius);
    let plain_bounds = InstanceBounds::new(&scene.meshes[once_skinned], Mat4::IDENTITY);
    assert_eq!(scene.instances[tile[42]].bounds.radius, plain_bounds.radius);
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn materials_that_look_alike_are_batched_together_whenever_they_were_made() {
    let adapter = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(Renderer::new_with(
        &adapter, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    let image = omsi_texture::Image { width: 1, height: 1, rgba: vec![200, 100, 50, 255], has_alpha: false };
    let texture = renderer.add_texture(&mut scene, &image, true);
    let a = renderer.add_material(&mut scene, Some(texture), AlphaMode::Opaque, [1.0; 4], false);
    renderer.prepare(&mut scene);
    let b = renderer.add_material(&mut scene, Some(texture), AlphaMode::Opaque, [1.0; 4], false);
    let c = renderer.add_material(&mut scene, Some(texture), AlphaMode::Opaque, [0.5, 0.5, 0.5, 1.0], false);
    let other = renderer.add_texture(&mut scene, &image, true);
    let d = renderer.add_material(&mut scene, Some(other), AlphaMode::Opaque, [1.0; 4], false);
    let look = |scene: &Scene, m: MaterialId| scene.materials[m].look;
    assert_ne!(scene.materials[a].bind_group, scene.materials[b].bind_group);
    assert_eq!(look(&scene, a), look(&scene, b));
    assert_ne!(look(&scene, a), look(&scene, c));
    assert_ne!(look(&scene, a), look(&scene, d));
    let data = MeshData {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        normals: vec![Vec3::Z; 3], uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2], ranges: vec![(0, 3, 0)], ..Default::default()
    };
    let mesh = renderer.add_mesh(&mut scene, &data);
    let instances: Vec<usize> = [a, c, b, d]
        .iter()
        .map(|&m| renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![m]))
        .collect();
    renderer.prepare(&mut scene);
    let mut items: Vec<DrawItem> = instances
        .iter()
        .map(|&i| {
            let material = scene.instances[i].materials[0];
            DrawItem { pipe: 0, mesh: mesh as u32, range: 0, material: material as u32, look: look(&scene, material), entry: scene.instances[i].base }
        })
        .collect();
    let (mut list, mut batches) = (Vec::new(), Vec::new());
    batch_items(&scene, &mut items, true, &mut list, &mut batches);
    assert_eq!(batches.len(), 3);
    let shared = batches.iter().find(|b| b.instances.len() == 2).expect("a and b in one batch");
    assert!(shared.material == a as u32 || shared.material == b as u32);
    let mut entries: Vec<u32> = list[shared.instances.start as usize..shared.instances.end as usize].to_vec();
    entries.sort_unstable();
    assert_eq!(entries, vec![scene.instances[instances[0]].base, scene.instances[instances[2]].base]);
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn cached_bounds_follow_transforms_skinning_and_recycled_resources() {
    let adapter = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let renderer = pollster::block_on(Renderer::new_with(
        &adapter, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
    )).expect("test renderer");
    let mut scene = renderer.new_scene();
    scene.cache_bounds = true;
    let material = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0; 4], true);
    let data = MeshData {
        positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
        normals: vec![Vec3::Z; 3], uvs: vec![glam::Vec2::ZERO; 3],
        indices: vec![0, 1, 2], ranges: vec![(0, 3, 0)], ..Default::default()
    };
    let mesh = renderer.add_mesh(&mut scene, &data);
    let origin = DVec3::new(1_000_000.000_001, 2_000_000.0, 12.0);
    let i = renderer.add_instance(&mut scene, mesh, origin, Mat4::IDENTITY, vec![material]);
    let check = |scene: &Scene, i: usize| {
        let inst = &scene.instances[i];
        let expected = InstanceBounds::new(&scene.meshes[inst.mesh], inst.transform);
        assert_eq!(Renderer::bounding_sphere(scene, inst),
            (expected.centre + (inst.origin - scene.render_origin).as_vec3(), expected.radius));
        assert_eq!(Renderer::instance_scale(scene, inst), expected.scale);
    };
    renderer.prepare(&mut scene);
    check(&scene, i);
    let transform = Mat4::from_scale_rotation_translation(
        Vec3::new(-2.0, 3.0, 4.0), glam::Quat::from_rotation_z(0.7), Vec3::new(10.0, 20.0, 30.0));
    renderer.set_transform(&mut scene, i, origin, transform);
    renderer.prepare(&mut scene);
    check(&scene, i);
    let posed: Vec<_> = data.positions.iter().map(|p| *p * 5.0 + Vec3::Z).collect();
    renderer.update_mesh(&mut scene, mesh, &posed, &data.normals, &data.uvs);
    renderer.prepare(&mut scene);
    check(&scene, i);
    renderer.set_render_origin(&mut scene, origin - DVec3::new(0.25, 0.5, 0.75));
    renderer.prepare(&mut scene);
    check(&scene, i);
    renderer.free_mesh(&mut scene, mesh);
    renderer.prepare(&mut scene);
    check(&scene, i);
    let new = renderer.add_mesh(&mut scene, &data);
    assert_eq!(renderer.recycle_mesh(&mut scene, new, mesh), mesh);
    renderer.prepare(&mut scene);
    check(&scene, i);
    let replacement = renderer.add_instance(&mut scene, mesh, origin, Mat4::IDENTITY, vec![material]);
    assert_eq!(renderer.recycle_instance(&mut scene, replacement, i), i);
    renderer.prepare(&mut scene);
    check(&scene, i);
    let other = renderer.add_mesh(&mut scene, &data);
    renderer.set_instance_mesh(&mut scene, i, other);
    renderer.prepare(&mut scene);
    check(&scene, i);
}

#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn presurface_reveals_excavation_before_terrain_is_drawn() {
    fn quad(renderer: &Renderer, scene: &mut Scene, y: f32, half: f32) -> MeshId {
        renderer.add_mesh(
            scene,
            &MeshData {
                positions: vec![
                    Vec3::new(-half, y, -half),
                    Vec3::new(half, y, -half),
                    Vec3::new(half, y, half),
                    Vec3::new(-half, y, half),
                ],
                normals: vec![-Vec3::Y; 4],
                uvs: vec![glam::Vec2::ZERO; 4],
                ranges: vec![(0, 6, 0)],
                indices: vec![0, 1, 2, 0, 2, 3],
                one_sided: false,
            },
        )
    }
    let camera = Camera {
        position: DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    for (msaa, ssao, enhanced) in [
        (1, false, false),
        (1, true, false),
        (1, true, true),
        (4, true, true),
    ] {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let mut renderer = pollster::block_on(Renderer::new_with(
            &instance,
            None,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            RenderOptions {
                msaa,
                ssao,
                shadow_size: 1024,
                fxaa: false,
                render_scale: 1.0,
                ..Default::default()
            },
        ))
        .expect("test renderer");
        renderer.profiling = true;
        let mut scene = renderer.new_scene();
        let green = renderer.add_material(
            &mut scene,
            None,
            AlphaMode::Opaque,
            [0.0, 1.0, 0.0, 1.0],
            true,
        );
        let blue = renderer.add_material(
            &mut scene,
            None,
            AlphaMode::Opaque,
            [0.0, 0.0, 1.0, 1.0],
            true,
        );
        let red = renderer.add_material(
            &mut scene,
            None,
            AlphaMode::Opaque,
            [1.0, 0.0, 0.0, 1.0],
            true,
        );
        let texture = renderer.add_texture(
            &mut scene,
            &omsi_texture::Image {
                width: 1,
                height: 1,
                rgba: vec![255, 255, 255, 0],
                has_alpha: true,
            },
            false,
        );
        let transparent =
            renderer.add_material(&mut scene, Some(texture), AlphaMode::Blend, [1.0; 4], true);
        let cutout =
            renderer.add_material(&mut scene, Some(texture), AlphaMode::Test, [1.0; 4], true);
        // Put terrain first in the scene to catch reliance on insertion order. The
        // excavation floor is behind it, with a transparent cover in front of both.
        let terrain = quad(&renderer, &mut scene, 6.0, 10.0);
        renderer.add_instance(
            &mut scene,
            terrain,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![green],
        );
        let floor = quad(&renderer, &mut scene, 8.0, 10.0);
        let floor = renderer.add_surface_instance(
            &mut scene,
            floor,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![blue],
        );
        scene.instances[floor].presurface = true;
        let cover_mesh = quad(&renderer, &mut scene, 4.0, 1.5);
        let cover = renderer.add_surface_instance(
            &mut scene,
            cover_mesh,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![transparent],
        );
        scene.instances[cover].presurface = true;
        // A normal opaque object lies between the excavation floor and its cover.
        // Prefilling its depth before the floor's colour erases the excavation, even
        // though the cover will reject that object's later colour draw.
        let background_mesh = quad(&renderer, &mut scene, 7.0, 2.0);
        renderer.add_instance(&mut scene, background_mesh, DVec3::ZERO, Mat4::IDENTITY, vec![red]);
        // Ordinary cutouts need shading to determine visibility on tile GPUs.
        // Keep a transparent one in the view to exercise mixed-scene prefilling.
        let foliage_mesh = quad(&renderer, &mut scene, 10.0, 0.25);
        renderer.add_instance(&mut scene, foliage_mesh, DVec3::ZERO, Mat4::IDENTITY, vec![cutout]);
        let foreground_mesh = quad(&renderer, &mut scene, 2.0, 0.25);
        let foreground = renderer.add_instance(
            &mut scene,
            foreground_mesh,
            DVec3::ZERO,
            Mat4::IDENTITY,
            vec![red],
        );
        scene.instances[foreground].visible = false;
        let lighting = Lighting {
            enhanced,
            shadows: false,
            fog_density: 0.0,
            ..Default::default()
        };
        let pixel = |rgba: &[u8], x: usize| -> [u8; 3] {
            rgba[(32 * 64 + x) * 4..(32 * 64 + x) * 4 + 3]
                .try_into()
                .unwrap()
        };
        let rgba = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        if enhanced && renderer.options.msaa > 1 {
            assert!(renderer.counts.borrow().get("msaa prepass batches").copied().unwrap_or(0.0) > 0.0,
                "an excavation must not disable depth prefilling for the rest of the view");
        }
        let saved = renderer.prepass_msaa_pipelines.take();
        let reference = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        renderer.prepass_msaa_pipelines = saved;
        assert!(rgba.iter().zip(&reference).all(|(a, b)| a.abs_diff(*b) <= 2),
            "presurface depth optimization changed the image: {msaa}/{ssao}/{enhanced}");
        let centre = pixel(&rgba, 32);
        assert!(
            centre[2] > centre[1] + 40,
            "floor must show through cover: {centre:?}; {msaa}/{ssao}/{enhanced}"
        );
        let outside = pixel(&rgba, 4);
        assert!(
            outside[1] > outside[2] + 40,
            "terrain outside cover: {outside:?}"
        );
        scene.instances[foreground].visible = true;
        Renderer::mark_changed(&mut scene, foreground);
        let rgba = renderer
            .render_to_image(&mut scene, 64, 64, &camera, &lighting)
            .unwrap();
        let centre = pixel(&rgba, 32);
        assert!(
            centre[0] > centre[2] + 40,
            "foreground stays visible: {centre:?}"
        );
        scene.instances[foreground].visible = false;
        Renderer::mark_changed(&mut scene, foreground);
        // Ordinary blended surfaces must keep showing the terrain, as must covers
        // whose material explicitly disables depth writes or uses alpha testing.
        for (presurface, alpha, no_z_write) in [
            (false, AlphaMode::Blend, false),
            (true, AlphaMode::Blend, true),
            (true, AlphaMode::Test, false),
        ] {
            scene.instances[cover].presurface = presurface;
            scene.materials[transparent].no_z_write = no_z_write;
            renderer.set_material(
                &mut scene,
                cover,
                0,
                if alpha == AlphaMode::Test {
                    cutout
                } else {
                    transparent
                },
            );
            let rgba = renderer
                .render_to_image(&mut scene, 64, 64, &camera, &lighting)
                .unwrap();
            let centre = pixel(&rgba, 32);
            assert!(
                centre[1] > centre[2] + 40,
                "terrain should show: {centre:?}; {presurface}/{alpha:?}/{no_z_write}"
            );
        }
    }
}

/// Two blended panes of one model, the near one listed first, both marked see-through
/// (`no_z_write`) for the shading: written into the depth buffer as Omsi.exe writes it
/// (`writes_depth`, no `[matl_noZwrite]` in the model), the far pane drawn after it is
/// hidden behind it; left out of it (`[matl_noZwrite]`), it is blended over it (#211).
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn stacked_panes_hide_each_other_in_model_order_where_they_write_depth() {
    let camera = Camera {
        position: DVec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
        roll: 0.0,
        fov_deg: 90.0,
        near: 0.1,
        far: 100.0,
    };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let quad = |y: f32, half: f32| -> [Vec3; 4] {
        [Vec3::new(-half, y, -half), Vec3::new(half, y, -half), Vec3::new(half, y, half), Vec3::new(-half, y, half)]
    };
    let wall = renderer.add_mesh(
        &mut scene,
        &MeshData {
            positions: quad(8.0, 20.0).to_vec(),
            normals: vec![-Vec3::Y; 4],
            uvs: vec![glam::Vec2::ZERO; 4],
            ranges: vec![(0, 6, 0)],
            indices: vec![0, 1, 2, 0, 2, 3],
            one_sided: false,
        },
    );
    let green = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.0, 1.0, 0.0, 1.0], true);
    renderer.add_instance(&mut scene, wall, DVec3::ZERO, Mat4::IDENTITY, vec![green]);
    // the near pane (slot 0) and the far one (slot 1), half transparent
    let panes = renderer.add_mesh(
        &mut scene,
        &MeshData {
            positions: [quad(2.0, 4.0), quad(4.0, 8.0)].concat(),
            normals: vec![-Vec3::Y; 8],
            uvs: vec![glam::Vec2::ZERO; 8],
            ranges: vec![(0, 6, 0), (6, 6, 1)],
            indices: vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7],
            one_sided: false,
        },
    );
    let half = renderer.add_texture(
        &mut scene,
        &omsi_texture::Image { width: 1, height: 1, rgba: vec![255, 255, 255, 128], has_alpha: true },
        false,
    );
    let near = renderer.add_material(&mut scene, Some(half), AlphaMode::Blend, [1.0, 0.0, 0.0, 1.0], true);
    let far = renderer.add_material(&mut scene, Some(half), AlphaMode::Blend, [0.0, 0.0, 1.0, 1.0], true);
    renderer.add_instance(&mut scene, panes, DVec3::ZERO, Mat4::IDENTITY, vec![near, far]);
    let lighting = Lighting { shadows: false, fog_density: 0.0, ..Default::default() };
    for writes_depth in [true, false] {
        for m in [near, far] {
            scene.materials[m].no_z_write = true;
            scene.materials[m].writes_depth = writes_depth;
        }
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        let c = &rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
        if writes_depth {
            assert!(c[2] < 20 && c[0] > 60 && c[1] > 60, "far pane hidden behind the near one: {c:?}");
        } else {
            assert!(c[2] > 40 && c[0] > 60, "far pane blended over the near one: {c:?}");
        }
    }
}

/// A `[matl_envmap]` reads its sphere map where Direct3D's D3DTSS_TCI_SPHEREMAP puts it
/// (u = Rx/m + 0.5, v = Ry/m + 0.5, m = 2|R - (0, 0, 1)| in camera space): a pane
/// turned 30 degrees from the view mirrors a ray 60 degrees off it, a quarter of the
/// way out from the map's middle (0.75), not near its rim (0.93) as the u = Rx/2 + 0.5
/// taken before had it; turned up, the same below the middle (the sky's half, #1193).
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn vanilla_sphere_map_is_read_where_direct3d_generates_it() {
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 100.0 };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    // the sphere map: red counts u, green v
    let size = 64usize;
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let at = |i: usize| ((i as f32 + 0.5) / size as f32 * 255.0).round() as u8;
            rgba.extend_from_slice(&[at(x), at(y), 0, 255]);
        }
    }
    let env = renderer.add_texture(&mut scene, &omsi_texture::Image { width: size as u32, height: size as u32, rgba, has_alpha: false }, false);
    let white = renderer.add_texture(&mut scene, &omsi_texture::Image { width: 1, height: 1, rgba: vec![255; 4], has_alpha: false }, false);
    let paint = renderer.add_material_env(&mut scene, Some(white), AlphaMode::Opaque, [1.0; 4], false, None, None, None, Some((env, 1.0)));
    // a pane 5 m ahead, its normal turned 30 degrees from the view: about z (a side
    // to the right) or about x (the pane tipped back, mirroring the sky)
    let pane = |renderer: &Renderer, scene: &mut Scene, n: Vec3, along: Vec3| {
        let c = Vec3::new(0.0, 5.0, 0.0);
        let across = n.cross(along);
        let mesh = renderer.add_mesh(
            scene,
            &MeshData {
                positions: vec![c - along - across, c + along - across, c + along + across, c - along + across],
                normals: vec![n; 4],
                uvs: vec![glam::Vec2::splat(0.5); 4],
                ranges: vec![(0, 6, 0)],
                indices: vec![0, 1, 2, 0, 2, 3],
                one_sided: false,
            },
        );
        renderer.add_instance(scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![paint])
    };
    let (s, c) = 30f32.to_radians().sin_cos();
    let lighting = Lighting { shadows: false, fog_density: 0.0, classic: true, ..Default::default() };
    let mut read = |n: Vec3, along: Vec3| {
        let id = pane(&renderer, &mut scene, n, along);
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        scene.instances[id].visible = false;
        let p = &rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
        (p[0] as f32 / 255.0, p[1] as f32 / 255.0)
    };
    let (u, v) = read(Vec3::new(s, -c, 0.0), Vec3::Z);
    assert!((u - 0.75).abs() < 0.03 && (v - 0.5).abs() < 0.03, "turned to the side: ({u}, {v})");
    let (u, v) = read(Vec3::new(0.0, -c, s), Vec3::X);
    assert!((u - 0.5).abs() < 0.03 && (v - 0.75).abs() < 0.03, "tipped back: ({u}, {v})");
}

/// A lamp's flare behind a window that writes its depth: seen from inside the vehicle
/// it shows through the glass, which Omsi.exe draws after the flares (0x6f0430 after
/// 0x6f0400/0x6f0418); seen from outside the vehicle, drawn before the flares, the
/// window hides it as in the original.
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn flares_show_through_the_glass_of_the_vehicle_the_camera_is_in() {
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 90.0, near: 0.1, far: 100.0 };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let quad = |y: f32, half: f32| MeshData {
        positions: vec![Vec3::new(-half, y, -half), Vec3::new(half, y, -half), Vec3::new(half, y, half), Vec3::new(-half, y, half)],
        normals: vec![-Vec3::Y; 4],
        uvs: vec![glam::Vec2::ZERO; 4],
        ranges: vec![(0, 6, 0)],
        indices: vec![0, 1, 2, 0, 2, 3],
        one_sided: false,
    };
    let wall = renderer.add_mesh(&mut scene, &quad(30.0, 60.0));
    let black = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.0, 0.0, 0.0, 1.0], true);
    renderer.add_instance(&mut scene, wall, DVec3::new(0.0, 0.0, 0.0), Mat4::IDENTITY, vec![black]);
    let pane = renderer.add_mesh(&mut scene, &quad(2.0, 4.0));
    let half = renderer.add_texture(
        &mut scene,
        &omsi_texture::Image { width: 1, height: 1, rgba: vec![255, 255, 255, 64], has_alpha: true },
        false,
    );
    let glass = renderer.add_material(&mut scene, Some(half), AlphaMode::Blend, [0.2, 0.2, 0.2, 1.0], true);
    scene.materials[glass].no_z_write = true;
    scene.materials[glass].writes_depth = true;
    renderer.add_instance(&mut scene, pane, DVec3::ZERO, Mat4::IDENTITY, vec![glass]);
    scene.coronas = vec![Corona { position: DVec3::new(0.0, 10.0, 0.0), size: 3.0, color: [1.0; 3], brightness: 1.0, ..Default::default() }];
    for inside in [true, false] {
        let lighting = Lighting {
            shadows: false,
            fog_density: 0.0,
            night: 1.0,
            inside: inside.then_some((DVec3::ZERO, 0.0, [6.0, 6.0, 6.0, 0.0, 0.0, 0.0])),
            ..Default::default()
        };
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        let c = &rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
        if inside {
            assert!(c[0] > 120, "the flare shows through the windscreen: {c:?}");
        } else {
            assert!(c[0] < 80, "the flare behind a bus's window seen from outside: {c:?}");
        }
    }
}

/// Rails laid a millimetre over a road as a spline of their own, seen from the cab: the
/// road's 10 m stretch between two cross-sections and the rails' metre-long ones are
/// pulled towards the eye alike, so the rails lie on the road right up to the bus. With a
/// fixed 2 cm pull per vertex the road's long triangles bowed over them near the eye and
/// the rails went under the road there (#1196).
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn a_spline_a_millimetre_over_a_road_stays_over_it_near_the_eye() {
    let camera = Camera { position: DVec3::new(0.0, 0.0, 2.5), yaw: 0.0, pitch: -25.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 200.0 };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer");
    let mut scene = renderer.new_scene();
    let strip = |half: f32, z: f32, stations: usize| {
        let mut m = MeshData { ranges: vec![(0, 6 * stations as u32, 0)], ..Default::default() };
        for i in 0..=stations {
            let y = 2.0 + 10.0 * i as f32 / stations as f32;
            m.positions.extend([Vec3::new(-half, y, z), Vec3::new(half, y, z)]);
            m.normals.extend([Vec3::Z; 2]);
            m.uvs.extend([glam::Vec2::ZERO; 2]);
        }
        for i in 0..stations as u32 {
            let a = 2 * i;
            m.indices.extend([a, a + 1, a + 2, a + 1, a + 3, a + 2]);
        }
        m
    };
    let road = renderer.add_mesh(&mut scene, &strip(3.0, 0.0, 1));
    let rails = renderer.add_mesh(&mut scene, &strip(0.2, 0.001, 10));
    let grey = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [0.3, 0.3, 0.3, 1.0], true);
    let red = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0, 0.0, 0.0, 1.0], true);
    for (mesh, mat) in [(road, grey), (rails, red)] {
        let i = renderer.add_surface_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![mat]);
        scene.instances[i].render_phase = RenderPhase::Spline;
    }
    let lighting = Lighting { shadows: false, fog_density: 0.0, ..Default::default() };
    let rgba = renderer.render_to_image(&mut scene, 128, 128, &camera, &lighting).unwrap();
    // the middle column from 3 m (row 93) to 11 m (row 40) in front of the camera
    for row in 42..92 {
        let c = &rgba[(row * 128 + 64) * 4..(row * 128 + 64) * 4 + 3];
        assert!(c[0] > c[1] + 60, "row {row}: the road over the rails: {c:?}");
    }
}

/// A `[smoke]` puff (an exhaust's, a wheel's spray) is drawn where it is, in its colour,
/// in every graphics mode (#949, #948).
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn smoke_puffs_are_drawn() {
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 100.0 };
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer");
    renderer.set_smoke_texture(&omsi_texture::Image { width: 4, height: 4, rgba: vec![255; 64], has_alpha: true });
    let mut scene = renderer.new_scene();
    for (classic, enhanced) in [(true, false), (false, false), (false, true)] {
        scene.smoke = vec![SmokeParticle { position: DVec3::new(0.0, 5.0, 0.0), size: 1.0, color: [1.0, 0.0, 0.0], alpha: 1.0, ..Default::default() }];
        let lighting = Lighting { shadows: false, fog_density: 0.0, classic, enhanced, ..Default::default() };
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        let c = &rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
        assert!(c[0] > 120 && c[1] < 60 && c[2] < 60, "a red puff 5 m ahead (classic {classic}, enhanced {enhanced}): {c:?}");
    }
}

fn smoke_test_renderer() -> Renderer {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 1, ssao: false, shadow_size: 1024, fxaa: false, render_scale: 1.0, ..Default::default() },
    ))
    .expect("test renderer")
}

/// A puff that knows the ground under it fades out into it over its lowest part - no
/// line where the road would cut it off - and keeps its colour higher up, in every
/// graphics mode; one wholly under its ground is not drawn.
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn a_smoke_puff_fades_into_its_ground() {
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 100.0 };
    let mut renderer = smoke_test_renderer();
    renderer.set_smoke_texture(&omsi_texture::Image { width: 4, height: 4, rgba: vec![255; 64], has_alpha: true });
    let mut scene = renderer.new_scene();
    let red = |row: usize, rgba: &[u8]| rgba[(row * 64 + 32) * 4] as i32;
    for (classic, enhanced) in [(true, false), (false, false), (false, true)] {
        let lighting = Lighting { shadows: false, fog_density: 0.0, classic, enhanced, ..Default::default() };
        // 5 m ahead with the ground at its middle: drawn 0.9 m round, a pixel row 9 cm
        // there (60 degrees over 64 rows); the fade reaches 0.18 m up
        let profile = |scene: &mut Scene, renderer: &mut Renderer| -> Vec<i32> {
            let rgba = renderer.render_to_image(scene, 64, 64, &camera, &lighting).unwrap();
            let bg = red(56, &rgba);
            (24..=40).map(|row| red(row, &rgba) - bg).collect()
        };
        scene.smoke = vec![SmokeParticle { position: DVec3::new(0.0, 5.0, 0.0), size: 1.0, color: [1.0, 0.0, 0.0], alpha: 1.0, ..Default::default() }];
        let plain = profile(&mut scene, &mut renderer);
        scene.smoke[0].ground = Some(0.0);
        let faded = profile(&mut scene, &mut renderer);
        let (at, full) = (|row: usize| faded[row - 24], |row: usize| plain[row - 24]);
        let mode = format!("classic {classic}, enhanced {enhanced}: {faded:?} against {plain:?}");
        assert!(full(25) > 30 && full(33) > 30, "the plain puff is drawn ({mode})");
        assert!(at(34).abs() < 4 && at(38).abs() < 4, "under its ground ({mode})");
        assert!(at(31) * 2 < full(31), "a few centimetres over the ground it is all but gone ({mode})");
        assert!((at(28) - full(28)).abs() <= 3 && (at(25) - full(25)).abs() <= 3, "30 cm up it is as it was ({mode})");
        for row in 28..31 {
            assert!(at(row) >= at(row + 1) - 2, "fading down into the ground, no step ({mode})");
        }
        // and one sunk wholly into its ground is left out
        scene.smoke[0].ground = Some(0.95);
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        assert!((red(30, &rgba) - red(56, &rgba)).abs() < 6, "a puff under its ground drawn (classic {classic}, enhanced {enhanced})");
    }
}

/// A puff's picture is turned by its spin about the line of sight (Omsi.exe turns every
/// one its own way): the top half of rauch.tga comes to the bottom at half a turn.
#[test]
#[ignore = "requires a graphics adapter; run with --ignored on a GPU host"]
fn a_smoke_puffs_picture_turns_with_its_spin() {
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 100.0 };
    let mut renderer = smoke_test_renderer();
    let mut rgba = vec![255u8; 64];
    for px in 8..16 {
        rgba[px * 4 + 3] = 0;
    }
    renderer.set_smoke_texture(&omsi_texture::Image { width: 4, height: 4, rgba, has_alpha: true });
    let mut scene = renderer.new_scene();
    let red = |row: usize, rgba: &[u8]| rgba[(row * 64 + 32) * 4] as i32;
    let lighting = Lighting { shadows: false, fog_density: 0.0, classic: true, ..Default::default() };
    for (spin, top) in [(0.0, true), (std::f32::consts::PI, false)] {
        scene.smoke = vec![SmokeParticle { position: DVec3::new(0.0, 5.0, 0.0), size: 1.0, color: [1.0, 0.0, 0.0], alpha: 1.0, spin, ..Default::default() }];
        let rgba = renderer.render_to_image(&mut scene, 64, 64, &camera, &lighting).unwrap();
        let bg = red(56, &rgba);
        let (up, down) = (red(25, &rgba) - bg, red(39, &rgba) - bg);
        assert_eq!((up > 60, down > 60), (top, !top), "spin {spin}: above {up}, below {down}");
    }
}

/// A smoke particle's sprite: its spin, its ground relative to the render origin and its
/// z offset where the smoke branch of the shader reads them; none when it is too faint or
/// wholly under its ground.
#[test]
fn a_smoke_sprite_carries_its_spin_ground_and_z_offset() {
    let ro = DVec3::new(1000.0, 2000.0, 30.0);
    let p = SmokeParticle { position: ro + DVec3::new(1.0, 2.0, 3.5), size: 1.0, color: [0.5; 3], alpha: 0.8, spin: std::f32::consts::FRAC_PI_2, z_offset: 0.1, ground: Some(33.0), ..Default::default() };
    let g = smoke_sprite(&p, ro).unwrap();
    assert_eq!(g.pos, [1.0, 2.0, 3.5]);
    assert_eq!(g.extra[3], 3.0);
    assert!(g.dir[0].abs() < 1e-6 && (g.dir[1] - 1.0).abs() < 1e-6, "{:?}", g.dir);
    assert_eq!(g.up[..3], [3.0, 1.0, 0.1]);
    assert!((g.up[3] - 0.18).abs() < 1e-6, "fade {}", g.up[3]);
    // the fade into the ground: a fifth of the drawn radius, 6 to 25 cm
    assert_eq!((smoke_ground_fade(0.1), smoke_ground_fade(5.0)), (0.06, 0.25));
    // no ground: not faded
    let free = smoke_sprite(&SmokeParticle { ground: None, ..p }, ro).unwrap();
    assert_eq!(free.up[1], 0.0);
    // its top (drawn at 0.9 of its half width) under the ground: left out
    assert!(smoke_sprite(&SmokeParticle { ground: Some(30.0 + 3.5 + 0.91), ..p }, ro).is_none());
    assert!(smoke_sprite(&SmokeParticle { ground: Some(30.0 + 3.5 + 0.89), ..p }, ro).is_some());
    assert!(smoke_sprite(&SmokeParticle { alpha: 0.0, ..p }, ro).is_none());
}

/// The launcher's preview (`RenderOptions::preview_only`) makes no enhanced pipeline,
/// reflection probe, ray tracer or cloud noise, its camera group leaves the enhanced
/// textures out, and it draws a frame with that group.
#[test]
fn launcher_preview_initializes_without_enhanced_or_ray_tracing_resources() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
    let instance = wgpu::Instance::new(descriptor);
    let mut renderer = pollster::block_on(Renderer::new_with(&instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb), RenderOptions { msaa: 1, preview_only: true, no_enhanced: true, ray_tracing: false, ..Default::default() })).unwrap();
    assert!(renderer.hdr_pass.is_none());
    assert!(renderer.probe.is_none());
    assert!(renderer.rt.is_none());
    assert!(renderer.cloud_shape_cpu.is_empty());
    let bindings: Vec<u32> = camera_layout_entries(ArrayPath::Storage, true).into_iter().map(|e| e.binding).collect();
    for binding in ENHANCED_CAMERA_TEXTURES {
        assert!(!bindings.contains(&binding), "preview camera layout must omit enhanced binding {binding}");
    }
    let mut scene = renderer.new_scene();
    let camera = Camera { position: DVec3::new(0.0, -35.0, 30.0), yaw: 0.0, pitch: -40.0, roll: 0.0, fov_deg: 60.0, near: 0.1, far: 1000.0 };
    renderer.render_to_image(&mut scene, 16, 16, &camera, &Lighting::default()).unwrap();
}

#[test]
fn noop_backend_initializes_renderer() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
    let instance = wgpu::Instance::new(descriptor);
    let res = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 2,
            shadow_size: 1024,
            ..Default::default()
        },
    ));
    assert!(res.is_ok(), "renderer should initialize on noop backend: {:?}", res.err());
}

/// The GL fallback must omit probe resources and match its camera bind group to the
/// vanilla pipelines. Exercise the full build and a frame, rather than the policy alone.
#[test]
fn vanilla_gl_build_omits_unused_enhanced_resources() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
    let instance = wgpu::Instance::new(descriptor);
    let base = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            shadow_size: 256,
            ..Default::default()
        },
    ))
    .unwrap();
    let mut renderer = Renderer::build(
        base.device.clone(),
        base.queue.clone(),
        "OpenGL regression adapter".into(),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        RenderOptions {
            msaa: 1,
            shadow_size: 256,
            no_enhanced: true,
            ..Default::default()
        },
    );
    assert!(renderer.hdr_pass.is_none());
    assert!(renderer.probe.is_none());
    assert!(renderer.cloud_shape_cpu.is_empty());
    let mut scene = renderer.new_scene();
    let camera = Camera {
        position: DVec3::new(0.0, -35.0, 30.0),
        yaw: 0.0,
        pitch: -40.0,
        roll: 0.0,
        fov_deg: 60.0,
        near: 0.1,
        far: 1000.0,
    };
    let scope = renderer
        .device
        .push_error_scope(wgpu::ErrorFilter::Validation);
    renderer
        .render_to_image(&mut scene, 16, 16, &camera, &Lighting::default())
        .unwrap();
    assert!(pollster::block_on(scope.pop()).is_none());
    // A desktop's native backend still prepares Enhanced for changing modes in-game.
    assert!(base.hdr_pass.is_some());
    assert!(base.probe.is_some());
}

#[test]
fn excavation_keeps_ordinary_depth_prefilling_without_a_gpu() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
    let instance = wgpu::Instance::new(descriptor);
    let mut renderer = pollster::block_on(Renderer::new_with(
        &instance, None, Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions { msaa: 4, ssao: true, render_scale: 1.0, ..Default::default() },
    )).expect("noop renderer");
    renderer.profiling = true;
    let mut scene = renderer.new_scene();
    let material = renderer.add_material(&mut scene, None, AlphaMode::Opaque, [1.0; 4], true);
    let mesh = renderer.add_mesh(&mut scene, &MeshData {
        positions: vec![Vec3::new(-2.0, 4.0, -2.0), Vec3::new(2.0, 4.0, -2.0),
            Vec3::new(2.0, 4.0, 2.0), Vec3::new(-2.0, 4.0, 2.0)],
        normals: vec![-Vec3::Y; 4], uvs: vec![glam::Vec2::ZERO; 4],
        indices: vec![0, 1, 2, 0, 2, 3], ranges: vec![(0, 6, 0)],
        ..Default::default()
    });
    let floor = renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![material]);
    scene.instances[floor].presurface = true;
    let transparent = renderer.add_material(&mut scene, None, AlphaMode::Blend, [1.0, 1.0, 1.0, 0.0], true);
    let cover = renderer.add_surface_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![transparent]);
    scene.instances[cover].presurface = true;
    let glass_material = renderer.add_material(&mut scene, None, AlphaMode::Blend, [1.0, 1.0, 1.0, 0.25], true);
    let glass = renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![glass_material]);
    let scenery = renderer.add_instance(&mut scene, mesh, DVec3::ZERO, Mat4::IDENTITY, vec![material]);
    let camera = Camera { position: DVec3::ZERO, yaw: 0.0, pitch: 0.0, roll: 0.0,
        fov_deg: 90.0, near: 0.1, far: 100.0 };
    let lighting = Lighting { enhanced: true, shadows: false, fog_density: 0.0, ..Default::default() };
    // Exercise the actual render planner on CI's CPU-only backend. A presurface
    // must not turn off prefilling for ordinary scenery, but ground must stay out:
    // its authored blends have to finish before later surface coverage writes depth.
    for phase in RenderPhase::DRAW_ORDER {
        scene.instances[scenery].render_phase = phase;
        renderer.counts.borrow_mut().clear();
        renderer.render_to_image(&mut scene, 16, 16, &camera, &lighting).unwrap();
        let batches = renderer.counts.borrow().get("msaa prepass batches").copied().unwrap_or(0.0);
        let ordinary = matches!(phase, RenderPhase::BeforeNormal | RenderPhase::Normal
            | RenderPhase::AfterNormal | RenderPhase::AfterVehicles);
        assert_eq!(batches > 0.0, ordinary, "phase {phase:?}");
    }
    // The explicit presurface flag overrides even an ordinary authored phase.
    scene.instances[scenery].render_phase = RenderPhase::Normal;
    scene.instances[scenery].presurface = true;
    renderer.counts.borrow_mut().clear();
    renderer.render_to_image(&mut scene, 16, 16, &camera, &lighting).unwrap();
    assert_eq!(renderer.counts.borrow().get("msaa prepass batches"), None);
    if cfg!(target_vendor = "apple") {
        scene.instances[scenery].presurface = false;
        scene.instances[glass].visible = false;
        renderer.counts.borrow_mut().clear();
        renderer.render_to_image(&mut scene, 16, 16, &camera, &lighting).unwrap();
        assert_eq!(renderer.counts.borrow().get("msaa prepass batches"), None,
            "an opaque-only view keeps native hidden-surface removal even with an excavation");
    }
}

#[test]
fn declared_transmap_ignores_slot_alpha() {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::NOOP;
    descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
    let instance = wgpu::Instance::new(descriptor);
    let renderer = pollster::block_on(Renderer::new_with(
        &instance,
        None,
        Some(wgpu::TextureFormat::Rgba8UnormSrgb),
        RenderOptions {
            msaa: 1,
            shadow_size: 1024,
            ..Default::default()
        },
    ))
    .expect("noop renderer");
    let mut scene = renderer.new_scene();
    let blended =
        |scene: &mut Scene, transmap: Option<(TextureId, bool)>, extra: MaterialExtra| {
            renderer.add_material_extra(
                scene,
                None,
                AlphaMode::Blend,
                [1.0; 4],
                true,
                transmap,
                None,
                None,
                None,
                [0.0; 3],
                extra,
            )
        };
    // declared, its file missing: no transmap texture bound
    let declared = blended(
        &mut scene,
        None,
        MaterialExtra {
            transmap_declared: true,
            ..Default::default()
        },
    );
    let map = renderer.add_blank_texture(&mut scene, 1, 1);
    let bound = blended(&mut scene, Some((map, true)), MaterialExtra::default());
    // another bit of the same flags, no transmap
    let metal = blended(
        &mut scene,
        None,
        MaterialExtra {
            metal_ok: true,
            ..Default::default()
        },
    );
    let plain = blended(&mut scene, None, MaterialExtra::default());
    assert_eq!(scene.materials[metal].uniform.params2[3], 4.0);
    let flags: Vec<bool> = [declared, bound, metal, plain]
        .iter()
        .map(|&m| scene.materials[m].transmap_declared())
        .collect();
    assert_eq!(flags, [true, true, false, false]);
    let corner = [Vec3::ZERO, Vec3::X, Vec3::Y];
    let data = MeshData {
        positions: corner.repeat(4),
        normals: vec![Vec3::Z; 12],
        uvs: vec![glam::Vec2::ZERO; 12],
        indices: (0..12).collect(),
        ranges: (0..4).map(|s| (s * 3, 3, s)).collect(),
        ..Default::default()
    };
    let mesh = renderer.add_mesh(&mut scene, &data);
    let i = renderer.add_instance(
        &mut scene,
        mesh,
        DVec3::ZERO,
        Mat4::IDENTITY,
        vec![declared, bound, metal, plain],
    );
    renderer.set_params(&mut scene, i, &[0.0, 0.0, 0.35, 0.35], true, &[]);
    assert_eq!(scene.instances[i].slot_alpha, vec![1.0, 1.0, 0.35, 0.35]);
}

#[test]
fn omsi_render_phases_are_monotonic_and_complete() {
    assert_eq!(
        RenderPhase::DRAW_ORDER.map(|phase| phase as usize),
        [0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
    assert_eq!(RenderPhase::default(), RenderPhase::Normal);
}

#[test]
fn metric_lifted_surfaces_keep_shading_class_without_view_space_pull() {
    assert_eq!(surface_instance_code(false, false, false, true, false), 0.9);
    assert_eq!(surface_instance_code(false, false, false, true, true), 1.0);
    assert_eq!(surface_instance_code(false, false, true, false, false), 0.9);
    assert_eq!(surface_instance_code(false, true, false, true, false), 0.75);
}

#[test]
fn spline_blend_sort_ignores_height_and_camera_pitch() {
    let origin = DVec3::new(120.0, 45.0, 0.0);
    let render_origin = DVec3::new(100.0, 40.0, 0.0);
    let level_camera = Vec3::new(3.0, 1.0, 4.0);
    let high_camera = Vec3::new(3.0, 1.0, 80.0);
    assert_eq!(
        horizontal_sort_distance(origin, render_origin, level_camera),
        horizontal_sort_distance(origin + DVec3::Z * 60.0, render_origin, high_camera)
    );
}

#[test]
fn render_scale_auto_keeps_ordinary_windows_sharp() {
    // the default window and a 2560x1080 screen are drawn at full size
    assert_eq!(scene_scale_for(0.0, 1600, 900), 1.0);
    assert_eq!(scene_scale_for(0.0, 2560, 1080), 1.0);
    // Mac/Android cap this at 2.8 million pixels; desktops keep it at full size.
    let s = scene_scale_for(0.0, 3200, 1800);
    if cfg!(target_os = "macos") || cfg!(target_os = "android") {
        assert!((s - 0.697).abs() < 0.01, "{s}");
        assert!((3200.0 * s * 1800.0 * s - AUTO_SCALE_PIXELS).abs() < 1.0);
    } else {
        assert_eq!(s, 1.0);
    }
    // never below half size
    assert_eq!(scene_scale_for(0.0, 16384, 16384), 0.5);
    // what is asked for, within 0.5..1 - on a Mac or a phone a scale over the pixel
    // budget is capped like the automatic one
    let s = scene_scale_for(0.75, 3200, 1800);
    if cfg!(target_os = "macos") || cfg!(target_os = "android") {
        assert!((s - 0.697).abs() < 0.01, "{s}");
        assert!((3200.0 * s * 1800.0 * s - AUTO_SCALE_PIXELS).abs() < 1.0);
    } else {
        assert_eq!(s, 0.75);
    }
    // a small window under the budget keeps what it asked for
    assert_eq!(scene_scale_for(0.3, 1600, 900), 0.5);
    assert_eq!(scene_scale_for(1.4, 1600, 900), 1.0);
}

#[test]
fn surface_depth_coverage_excludes_glass_and_terrain_masks() {
    assert!(surface_depth_coverage(RenderPhase::Spline, AlphaMode::Blend, false, false));
    for phase in RenderPhase::DRAW_ORDER {
        if !world_surface_phase(phase) {
            assert!(!surface_depth_coverage(phase, AlphaMode::Blend, false, false));
        } else {
            assert!(surface_depth_coverage(phase, AlphaMode::Blend, false, false));
        }
    }
    assert!(!surface_depth_coverage(RenderPhase::Spline, AlphaMode::Blend, true, false));
    assert!(!surface_depth_coverage(RenderPhase::Spline, AlphaMode::Blend, false, true));
    assert!(!surface_depth_coverage(RenderPhase::Spline, AlphaMode::Opaque, false, false));
    assert!(!surface_depth_coverage(RenderPhase::Spline, AlphaMode::Test, false, false));
    let order = RenderPhase::DRAW_ORDER;
    assert!(order.iter().position(|p| *p == RenderPhase::OnSurface).unwrap()
        < order.iter().position(|p| *p == RenderPhase::BeforeNormal).unwrap());
}

#[test]
fn pipeline_codes_cover_the_table() {
    // every kind x culling x depth bias has its own pipeline, and the opaque and
    // alpha-tested codes sort before the blended ones
    let mut seen = std::collections::HashSet::new();
    for kind in 0..PIPE_KINDS {
        for cull in [false, true] {
            for surface in [false, true] {
                let code = pipe_code(kind, cull, surface);
                assert!((code as usize) < PIPE_KINDS as usize * 4);
                assert!(seen.insert(code));
                assert_eq!(
                    code < pipe_code(PIPE_BLEND, false, false),
                    kind < PIPE_BLEND
                );
            }
        }
    }
    assert_eq!(seen.len(), PIPE_KINDS as usize * 4);
    assert_eq!(pipe_code(PIPE_OPAQUE, false, false), 0);
    assert!(
        pipe_code(PIPE_ALPHA_TEST, true, true) < pipe_code(PIPE_BLEND_NO_WRITE, false, false)
    );
}

#[test]
fn nearest_by_origin_picks_the_closest_mesh_of_each_object() {
    // A long bus (one origin) has a window 3 m from the camera and another 14 m away;
    // ranking the whole object by its shared origin (here 8 m) instead of its nearest
    // blended mesh is exactly what let a car beside the near window sort as farther
    // away than the bus and disappear behind it.
    let bus = DVec3::new(0.0, 0.0, 0.0);
    let car = DVec3::new(1.0, 0.0, 0.0);
    let by_origin = nearest_by_origin([(bus, 14.0), (bus, 3.0), (car, 8.0)]);
    let bus_dist = by_origin[&origin_key(bus)];
    let car_dist = by_origin[&origin_key(car)];
    assert_eq!(
        bus_dist, 3.0,
        "the object's distance is its nearest mesh, not the first or an average"
    );
    assert_eq!(car_dist, 8.0);
    // the car (8 m) is nearer than the bus's near window (3 m) is far: with the old
    // origin-only distance (bus origin 8.5 m, say) the two could tie or invert
    assert!(car_dist > bus_dist);
    assert_eq!(
        by_origin.len(),
        2,
        "one entry per distinct origin, not per mesh"
    );
}

#[test]
fn origin_lookup_keeps_float_equality_and_large_coordinates() {
    let a = DVec3::new(-0.0, 1_000_000.000_001, 2.0);
    let b = DVec3::new(0.0, a.y, 2.0);
    let c = DVec3::new(0.0, a.y + 0.000_001, 2.0);
    let distances = nearest_by_origin([(a, 8.0), (b, 3.0), (c, 5.0), (DVec3::NAN, 1.0)]);
    assert_eq!(distances.len(), 2);
    assert_eq!(distances[&origin_key(a)], 3.0);
    assert_eq!(distances[&origin_key(c)], 5.0);
}

/// Every shader module parses and validates as the device will see it, translates to
/// Metal and Vulkan SPIR-V, and its uniform structs are laid out as the Rust side writes them.
#[test]
fn shaders_validate_and_match_the_uniforms() {
    use wgpu::naga;
    let modules = [
        ("scene", scene_shader_source(false)),
        ("scene opaque", scene_shader_source(false)),
        ("scene terrain paint", scene_shader_source(false)),
        ("sky", sky_shader_source()),
        ("corona", corona_shader_source()),
        ("fog lamps", fog_lamps_shader_source()),
        ("snow", snow_shader_source()),
        ("post", include_str!("post.wgsl").to_string()),
        ("ssao", include_str!("ssao.wgsl").to_string()),
        ("puddles", puddles::shader_source()),
        ("upscale", include_str!("upscale.wgsl").to_string()),
        ("mip", include_str!("mip.wgsl").to_string()),
        ("xr_ui", include_str!("xr_ui.wgsl").to_string()),
        // Enhanced+: the scene's pass with its reflection targets, the ray tracing
        ("scene (Enhanced+)", scene_shader_source(false).replace("//RT ", "")),
        ("ray-traced lighting", rt::lighting_source()),
        ("ray-traced reflections", rt::reflect_source()),
        ("texture means", include_str!("rt_avg.wgsl").to_string()),
    ];
    let sizes: &[(&str, usize)] = &[
        ("Enhanced", std::mem::size_of::<EnhancedUniform>()),
        ("PostParams", std::mem::size_of::<PostUniform>()),
        ("FogLampParams", std::mem::size_of::<FogLampUniform>()),
        ("SnowParams", std::mem::size_of::<SnowUniform>()),
        ("PuddleParams", std::mem::size_of::<puddles::Uniform>()),
        ("VehicleReflection", std::mem::size_of::<puddles::VehicleUniform>()),
        ("PointLight", std::mem::size_of::<GpuPointLight>()),
        ("Camera", std::mem::size_of::<CameraUniform>()),
        ("MaterialParams", std::mem::size_of::<MaterialUniform>()),
        ("RtParams", rt::PARAMS_SIZE),
    ];
    let mut checked = std::collections::HashSet::new();
    for (name, src) in &modules {
        let module = naga::front::wgsl::parse_str(src)
            .unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(src)));
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        // Exercise the painted-ground specialization as well as the default shader.
        let mut constants = naga::back::PipelineConstants::default();
        if name.starts_with("scene ") {
            constants.insert("ALPHA_TEST".into(), 0.0);
        }
        if *name == "scene terrain paint" {
            constants.insert("TERRAIN_PAINT".into(), 1.0);
        }
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            None,
            &constants,
        )
        .unwrap_or_else(|e| panic!("{name}: overrides: {e:?}"));
        let (module, info) = (module.into_owned(), info.into_owned());
        #[cfg(target_os = "macos")]
        let options = naga::back::msl::Options {
            lang_version: (2, 4),
            ..Default::default()
        };
        #[cfg(target_os = "macos")]
        naga::back::msl::write_string(
            &module,
            &info,
            &options,
            &naga::back::msl::PipelineOptions::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: Metal: {e:?}"));
        for entry in &module.entry_points {
            let pipeline = naga::back::spv::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
            };
            naga::back::spv::write_vec(&module, &info, &Default::default(), Some(&pipeline))
                .unwrap_or_else(|e| panic!("{name}/{}: Vulkan: {e:?}", entry.name));
        }
        let mut layouter = naga::proc::Layouter::default();
        layouter.update(module.to_ctx()).expect("layout");
        for (ty_name, rust) in sizes {
            if *ty_name == "Camera" && *name == "corona" {
                // Corona owns just the prefix through `cam_up`.  It must still include
                // every member in that prefix: a missing `world_origin`, for example,
                // makes a subsequently-added lighting use read the wrong vec4.
                if let Some((h, _)) = module
                    .types
                    .iter()
                    .find(|(_, t)| t.name.as_deref() == Some(*ty_name))
                {
                    assert_eq!(
                        layouter[h].size as usize,
                        std::mem::offset_of!(CameraUniform, clouds),
                        "corona: Camera prefix"
                    );
                }
                continue;
            }
            if let Some((h, _)) = module
                .types
                .iter()
                .find(|(_, t)| t.name.as_deref() == Some(*ty_name))
            {
                assert_eq!(layouter[h].size as usize, *rust, "{name}: {ty_name}");
                checked.insert(*ty_name);
            }
        }
    }
    assert_eq!(checked.len(), sizes.len(), "structs checked: {checked:?}");
}

/// The scene module as the OpenGL backend gets it translates to GLSL ES 3.10 and desktop
/// GLSL 4.30 for every entry point, with no `invariant gl_FragCoord` (rejected by AMD's
/// desktop GL and by GLES) and no texture read through two samplers (#617, #610).
#[test]
fn the_scene_shader_translates_to_glsl() {
    use wgpu::naga;
    use wgpu::naga::back::glsl;
    let src = scene_shader_source(true);
    let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .expect("validate");
    let constants = [("ALPHA_TEST".into(), 0.0), ("TERRAIN_PAINT".into(), 1.0)]
        .into_iter().collect();
    let (module, info) =
        naga::back::pipeline_constants::process_overrides(&module, &info, None, &constants).expect("overrides");
    for version in [glsl::Version::Embedded { version: 310, is_webgl: false }, glsl::Version::Desktop(430)] {
        let options = glsl::Options { version, ..Default::default() };
        for entry in &module.entry_points {
            let pipeline = glsl::PipelineOptions {
                shader_stage: entry.stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut out = String::new();
            glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, Default::default())
                .and_then(|mut w| w.write())
                .unwrap_or_else(|e| panic!("{version:?} {}: {e:?}", entry.name));
            assert!(!out.contains("invariant gl_FragCoord"), "{version:?} {}", entry.name);
        }
    }
}

/// The corona and smoke module (its smoke branch with the fade into the ground among
/// them) translates to the GLSL of the oldest OpenGL chips the renderer runs on, GLES 3.0
/// and GL 3.3, as to GLES 3.1 and GL 4.3.
#[test]
fn the_corona_shader_translates_to_glsl() {
    use wgpu::naga;
    use wgpu::naga::back::glsl;
    let src = corona_shader_source();
    let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module)
        .expect("validate");
    let (module, info) =
        naga::back::pipeline_constants::process_overrides(&module, &info, None, &Default::default()).expect("overrides");
    let versions = [
        glsl::Version::Embedded { version: 300, is_webgl: false },
        glsl::Version::Desktop(330),
        glsl::Version::Embedded { version: 310, is_webgl: false },
        glsl::Version::Desktop(430),
    ];
    for version in versions {
        let options = glsl::Options { version, ..Default::default() };
        let mut done = 0;
        for entry in module.entry_points.iter().filter(|e| ["vs_main", "fs_main", "fs_smoke", "fs_smoke_enhanced"].contains(&e.name.as_str())) {
            let pipeline = glsl::PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone(), multiview: None };
            let mut out = String::new();
            glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, Default::default())
                .and_then(|mut w| w.write())
                .unwrap_or_else(|e| panic!("{version:?} {}: {e:?}", entry.name));
            done += 1;
        }
        assert_eq!(done, 4, "{version:?}");
    }
}

/// Where the device cannot read storage buffers in a vertex shader, or none at all, the
/// scene module reads its arrays from textures: it validates and translates to the GLSL
/// of such chips (GLES 3.1 / GL 4.3 with storage for the lights, GLES 3.0 / GL 3.3
/// without), and nothing of it is a storage buffer where it must not be (#770, #316).
#[test]
fn the_scene_shader_reads_its_arrays_from_textures_without_vertex_storage() {
    use wgpu::naga;
    use wgpu::naga::back::glsl;
    for (path, versions) in [
        (ArrayPath::VertexTextures, [glsl::Version::Embedded { version: 310, is_webgl: false }, glsl::Version::Desktop(430)]),
        (ArrayPath::NoStorage, [glsl::Version::Embedded { version: 300, is_webgl: false }, glsl::Version::Desktop(330)]),
    ] {
        let src = scene_shader_for(true, path);
        assert!(!src.contains("models[") && !src.contains("inst_params[") && !src.contains("draw_list["));
        assert_eq!(src.contains("var<storage"), path == ArrayPath::VertexTextures, "{path:?}");
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{path:?}: {}", e.emit_to_string(&src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{path:?}: {e:?}"));
        let (module, info) =
            naga::back::pipeline_constants::process_overrides(&module, &info, None, &Default::default()).expect("overrides");
        for version in versions {
            let options = glsl::Options { version, ..Default::default() };
            for entry in &module.entry_points {
                let pipeline = glsl::PipelineOptions { shader_stage: entry.stage, entry_point: entry.name.clone(), multiview: None };
                let mut out = String::new();
                glsl::Writer::new(&mut out, &module, &info, &options, &pipeline, Default::default())
                    .and_then(|mut w| w.write())
                    .unwrap_or_else(|e| panic!("{path:?} {version:?} {}: {e:?}", entry.name));
                let vertex = entry.stage == naga::ShaderStage::Vertex;
                if vertex || path == ArrayPath::NoStorage {
                    assert!(!out.contains(" buffer "), "{path:?} {version:?} {}: a storage block", entry.name);
                }
                if path == ArrayPath::NoStorage {
                    assert!(!out.contains("textureGather"), "{path:?} {version:?} {}: textureGather", entry.name);
                }
            }
        }
    }
    // the rewrite leaves names that only end alike alone, and nests
    assert_eq!(indexing_as_calls("a = my_models[1]; b = models[models[i + 1u] .x];", "models", "f"), "a = my_models[1]; b = f(f(i + 1u) .x);");
}

/// wgpu's OpenGL backend numbers the textures of all a pipeline's groups into sixteen
/// units: the scene's camera and material groups stay within them on every array path
/// there, and what is left out of the camera group for it no pipeline made there reads
/// (#1133, #1154, #1162, #1176).
#[test]
fn the_scene_groups_fit_the_sixteen_texture_units_of_opengl() {
    use wgpu::naga;
    let textures = |e: &[wgpu::BindGroupLayoutEntry]| e.iter().filter(|e| matches!(e.ty, wgpu::BindingType::Texture { .. })).count();
    let material = textures(&material_layout_entries());
    for path in [ArrayPath::Storage, ArrayPath::VertexTextures, ArrayPath::NoStorage] {
        let sixteen = path != ArrayPath::Storage;
        let n = textures(&camera_layout_entries(path, sixteen)) + material;
        assert!(n <= 16, "{path:?}: {n} textures");
    }
    // (and nothing is left out where storage buffers carry the arrays)
    assert_eq!(textures(&camera_layout_entries(ArrayPath::Storage, false)) + material, 16);
    // the entry points that read a texture left out are the enhanced path's, the
    // probe's, the puddles' and the reflection pass's - none made there
    let left_out = |src: &str| -> Vec<String> {
        let module = naga::front::wgsl::parse_str(src).unwrap();
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).unwrap();
        let mut out = Vec::new();
        for (i, ep) in module.entry_points.iter().enumerate() {
            let used = info.get_entry_point(i);
            let reads = module.global_variables.iter().any(|(h, g)| {
                !used[h].is_empty() && g.binding.as_ref().is_some_and(|b| b.group == 0 && ENHANCED_CAMERA_TEXTURES.contains(&b.binding))
            });
            if reads {
                out.push(ep.name.clone());
            }
        }
        out
    };
    let only_enhanced = |name: &str| name.contains("enhanced") || name.contains("probe") || name.contains("puddle") || name.contains("reflections") || name == "fs_sky_cube";
    for path in [ArrayPath::VertexTextures, ArrayPath::NoStorage] {
        let scene = arrays_as_textures(&scene_shader_text(true), path);
        for src in [scene, sky_shader_source(), corona_shader_source()] {
            for name in left_out(&src) {
                assert!(only_enhanced(&name), "{path:?}: {name} reads an enhanced texture");
            }
        }
    }
}

/// An array texture is written in the rest of a row, whole rows and the start of the
/// last row, and never past its end.
#[test]
fn array_texture_writes_cover_the_range_once() {
    let w = ARRAY_TEX_WIDTH;
    assert_eq!(array_tex_spans(0, 10, 4 * w as u64), vec![(0, 0, 10, 1)]);
    assert_eq!(array_tex_spans(w as u64 - 2, 5, 4 * w as u64), vec![(w - 2, 0, 2, 1), (0, 1, 3, 1)]);
    assert_eq!(array_tex_spans(5, 3 * w as u64, 4 * w as u64), vec![(5, 0, w - 5, 1), (0, 1, w, 2), (0, 3, 5, 1)]);
    assert_eq!(array_tex_spans(0, 2 * w as u64, 4 * w as u64), vec![(0, 0, w, 2)]);
    // past the end: cut
    assert_eq!(array_tex_spans(2 * w as u64 - 1, 10, 2 * w as u64), vec![(w - 1, 1, 1, 1)]);
    assert!(array_tex_spans(3 * w as u64, 10, 2 * w as u64).is_empty());
    for (at, n) in [(0u64, 1u64), (7, 9000), (2047, 2049), (4096, 4096)] {
        let covered: u64 = array_tex_spans(at, n, 64 * w as u64).iter().map(|s| s.2 as u64 * s.3 as u64).sum();
        assert_eq!(covered, n);
    }
}

#[test]
fn the_sky_is_recomputed_only_when_it_has_moved_on() {
    let a = atmosphere::SkyInput::default();
    assert!(!sky_input_differs(&a, &a));
    // the sun a hundredth of a degree on, a tint that drifts by a thousandth: the same sky
    let turn = |deg: f32| {
        glam::Quat::from_axis_angle(a.sun_dir.any_orthonormal_vector(), deg.to_radians())
            * a.sun_dir
    };
    assert!(!sky_input_differs(
        &a,
        &atmosphere::SkyInput {
            sun_dir: turn(0.01),
            tint: [Vec3::splat(1.001); 3],
            ..a
        }
    ));
    // a quarter of a degree, some rain, another envir.cfg: a new sky
    assert!(sky_input_differs(
        &a,
        &atmosphere::SkyInput {
            sun_dir: turn(0.25),
            ..a
        }
    ));
    assert!(sky_input_differs(
        &a,
        &atmosphere::SkyInput { rain: 0.3, ..a }
    ));
    assert!(sky_input_differs(
        &a,
        &atmosphere::SkyInput {
            tint: [Vec3::new(1.2, 1.0, 0.9), Vec3::ONE, Vec3::ONE],
            ..a
        }
    ));
}

#[test]
fn each_path_gets_its_own_headlights() {
    let lamp = PointLight {
        radius: 30.0,
        color: [1.0, 0.78, 0.46],
        core: 5.0,
        ..Default::default()
    };
    let stand_in = PointLight {
        radius: 18.0,
        intensity: 0.8,
        mode: LightMode::Vanilla,
        ..Default::default()
    };
    let spot = PointLight {
        radius: 60.0,
        intensity: 20.0,
        direction: Vec3::new(0.0, 2.0, -0.6),
        cone: [0.97, 0.82],
        core: 1.0,
        beam: 24.0,
        mode: LightMode::Enhanced,
        ..Default::default()
    };
    assert!(drawn_by(&lamp, false) && drawn_by(&lamp, true));
    assert!(drawn_by(&stand_in, false) && !drawn_by(&stand_in, true));
    assert!(!drawn_by(&spot, false) && drawn_by(&spot, true));
    assert!(!drawn_by(
        &PointLight {
            intensity: 0.0,
            ..lamp
        },
        true
    ));
    // a map light reads as before in the vanilla shader (position, radius, colour)
    let g = gpu_light(&lamp, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(g.pos, [1.0, 2.0, 3.0, 30.0]);
    assert_eq!(g.color, [1.0, 0.78, 0.46, 1.0]);
    assert_eq!(g.dir[3], -2.0);
    assert_eq!(g.extra[1..], [5.0, 0.0, 30.0]);
    // a spot: no radius for the vanilla shader, the cone and the range for the enhanced
    let g = gpu_light(&spot, Vec3::ZERO);
    assert_eq!(g.pos[3], 0.0);
    assert_eq!(g.extra[3], 60.0);
    assert!(
        (Vec3::from_slice(&g.dir[..3]).length() - 1.0).abs() < 1e-5
            && g.dir[3] == 0.82
            && g.extra[0] == 0.97
    );
    // a headlight's beam gain rides along; a plain lamp has none
    assert_eq!(g.extra[2], 24.0);
    assert_eq!(gpu_light(&lamp, Vec3::ZERO).extra[2], 0.0);
}

#[test]
fn metering_defaults_are_gentle() {
    // the light model's exposure leads: the metering corrects well under a stop either
    // way, so snow stays white and a night stays dark
    let m = meter_tuning();
    assert!(m[0] > 0.0 && m[0] < 0.6, "{m:?}");
    assert!(m[2] <= 2.5 && m[3] <= 2.0, "{m:?}");
    // a snow field metered 1.5 EV over the target is darkened by well under a stop
    let ev = ((m[1] - (m[1] + 1.5)) * m[0]).clamp(-m[2], m[3]);
    assert!(ev < 0.0 && ev >= -0.75, "{ev}");
}

#[test]
fn half_floats_read_back() {
    for v in [0.0f32, 1.0, -2.5, 0.5, 1e-3, -3.46] {
        assert!(
            (half_to_f32(atmosphere::f16_bits(v)) - v).abs() <= v.abs() * 1e-3 + 1e-6,
            "{v}"
        );
    }
}

#[test]
fn vehicle_box_contains_the_driver() {
    // an NL202: 2.5 x 12 x 3 m, the box centre 0.4 m ahead of the origin and 1.5 m up,
    // heading east; the driver sits 4.6 m ahead of the origin, 1.8 m up
    let bus = (
        DVec3::new(100.0, 200.0, 30.0),
        90.0,
        [2.5, 12.0, 3.0, 0.0, 0.4, 1.5],
    );
    assert!(point_in_vehicle_box(DVec3::new(104.6, 200.7, 31.8), &bus));
    // beside the bus, above it, behind it
    assert!(!point_in_vehicle_box(DVec3::new(104.6, 197.0, 31.8), &bus));
    assert!(!point_in_vehicle_box(DVec3::new(104.6, 200.0, 34.0), &bus));
    assert!(!point_in_vehicle_box(DVec3::new(93.0, 200.0, 31.0), &bus));
}

#[test]
fn opaque_and_transmapped_materials_ignore_dynamic_alpha() {
    assert_eq!(
        Renderer::clamp_slot_alpha(0.0, AlphaMode::Opaque, false),
        1.0
    );
    assert_eq!(
        Renderer::clamp_slot_alpha(0.35, AlphaMode::Opaque, false),
        1.0
    );
    assert_eq!(Renderer::clamp_slot_alpha(0.0, AlphaMode::Test, false), 1.0);
    assert_eq!(
        Renderer::clamp_slot_alpha(0.35, AlphaMode::Test, false),
        1.0
    );
    assert_eq!(
        Renderer::clamp_slot_alpha(0.85, AlphaMode::Blend, false),
        0.85
    );
    assert_eq!(Renderer::clamp_slot_alpha(0.0, AlphaMode::Blend, true), 1.0);
    assert_eq!(
        Renderer::clamp_slot_alpha(0.0, AlphaMode::Opaque, true),
        1.0
    );
    assert_eq!(Renderer::clamp_slot_alpha(0.0, AlphaMode::Test, true), 1.0);
}
