//! Precipitation: rain streaks / snow flakes falling around the camera, drawn through the
//! corona sprite pipeline (cone value -2 marks a streak), and the picture the film on the
//! glass wears in a snow weather.

use glam::{DVec3, Vec3};
use omsi_render::{Corona, Scene};

const HALF_WIDTH: f32 = 16.0;
/// The box round the eye the near drops fill: as much rain as the eye can tell apart is
/// close by (a 2 mm drop 15 m away is a tenth of a pixel), so the near ones are as many as
/// the far again in a fraction of the room, and a downpour is a curtain, not a few lines.
const NEAR_HALF_WIDTH: f32 = 7.0;
const NEAR_HEIGHT: f32 = 12.0;

/// How long the eye sees a falling drop as one streak (s): the length of a streak is the
/// way the drop goes in that time against the eye - short and upright standing, long and
/// slanting at the windscreen of a bus at speed (#1412).
const STREAK_TIME: f32 = 1.0 / 30.0;

/// The rain rate (mm/h) of OMSI's precipitation strength (0..1 of its 255): a drizzle at
/// the bottom, a cloudburst (60 mm/h) at the top.
pub fn rain_rate_mm_h(rate: f32) -> f32 {
    0.3 + 60.0 * rate.clamp(0.0, 1.0).powi(2)
}

/// The median drop diameter (mm) of rain falling at `r` mm/h: Marshall and Palmer's drop
/// sizes (1948), `N(D) = 8000 e^(-ΛD)` with `Λ = 4.1 R^-0.21` per mm, whose median volume
/// diameter is 3.67 / Λ.
pub fn median_drop_mm(r: f32) -> f32 {
    3.67 / (4.1 * r.max(0.01).powf(-0.21))
}

/// The speed (m/s) a raindrop of `d` mm falls at in still air at sea level (Atlas, Srivastava
/// and Sekhon 1973): a drizzle drifts down at 2-4 m/s, the big drops of a cloudburst at 7-8.
pub fn terminal_speed(d: f32) -> f32 {
    (9.65 - 10.3 * (-0.6 * d.max(0.0)).exp()).max(0.5)
}

/// The rain's quality of a `rain_quality` setting (`high`, `medium`, `low`): 2 high, 1
/// medium, 0 low.
pub fn quality_of(q: &str) -> u8 {
    match q {
        "low" => 0,
        "medium" => 1,
        _ => 2,
    }
}

pub fn weather_wind(weather: &omsi_content::weather::Weather) -> Vec3 {
    let heading = weather.wind.0.to_radians();
    Vec3::new(heading.sin(), heading.cos(), 0.0) * weather.wind.1
}

pub struct Rain {
    /// The far drops (`..near_from`) in the wide box, the near ones after them.
    particles: Vec<Vec3>,
    near_from: usize,
    kind: i32,
    rate: f32,
    /// How fast the drops fall (m/s) and how thick they are (mm), from the rain's rate.
    fall: f32,
    drop_mm: f32,
    rng: u64,
    camera: Option<DVec3>,
}

/// The boxes a vehicle keeps the weather out of, as `Rain::tick` takes them: its own
/// `[boundingbox]` and each coupled part's (an articulated bus's rear section is a part of
/// its own, with its own box: without it, it rained in the rear saloon, #777).
pub fn vehicle_boxes(v: &omsi_sim::VehicleInstance) -> Vec<(DVec3, f64, [f32; 6])> {
    let front = v.ty.def.bounding_box.map(|bb| (v.position, v.heading, bb));
    let parts = v.trailers.iter().filter_map(|t| {
        // a part coupled the other way round stands turned about its own origin
        let heading = if t.reversed { t.heading + 180.0 } else { t.heading };
        t.ty.def.bounding_box.map(|bb| (t.position, heading, bb))
    });
    front.into_iter().chain(parts).collect()
}

impl Rain {
    pub fn new() -> Rain {
        Rain {
            particles: Vec::new(),
            near_from: 0,
            kind: 0,
            rate: 0.0,
            fall: 9.0,
            drop_mm: 2.0,
            rng: 0xABCDEF12345,
            camera: None,
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    /// `kind`: 0 none, 1 rain, 2 snow; `rate` 0..1; `quality` the rain's (see [`quality_of`]).
    pub fn set(&mut self, kind: i32, rate: f32, quality: u8) {
        self.kind = kind;
        self.rate = rate.clamp(0.0, 1.0);
        // the drops of this rain: small and slow in a drizzle, big and fast in a downpour
        // (they all looked the same whatever the rain, #1412)
        self.drop_mm = median_drop_mm(rain_rate_mm_h(self.rate));
        self.fall = terminal_speed(self.drop_mm);
        // (snow is drawn by the renderer itself, from the weather: render snow.wgsl,
        // `Lighting::snowfall`); no particles either while nothing falls
        let (far, near) = if kind == 0 || kind == 2 || self.rate == 0.0 {
            (0, 0)
        } else {
            // (fewer streaks at a lower rain quality: they are drawn as sprites each frame)
            let share = [0.25, 0.5, 1.0][quality.min(2) as usize];
            (((300.0 + 2700.0 * self.rate) * share) as usize, ((200.0 + 2800.0 * self.rate) * share) as usize)
        };
        if self.particles.len() != far + near || self.near_from != far {
            self.particles.clear();
            for i in 0..far + near {
                let (half, height) = if i < far { (HALF_WIDTH, 20.0) } else { (NEAR_HALF_WIDTH, NEAR_HEIGHT) };
                let p = Vec3::new(self.rand() * (half * 2.0) - half, self.rand() * (half * 2.0) - half, self.rand() * height);
                self.particles.push(p);
            }
            self.near_from = far;
        }
    }

    /// Move the drops by `dt`; the drops' velocity against the eye (m/s).
    fn advance(&mut self, dt: f32, camera: DVec3, wind: Vec3) -> Vec3 {
        // Keep drops in the world as the camera moves through them. Recentring the box
        // used to carry every drop along with the bus, even at motorway speed.
        let movement = self.camera.replace(camera).map_or(Vec3::ZERO, |old| (camera - old).as_vec3());
        let velocity = wind - Vec3::Z * if self.kind == 2 { 1.5 } else { self.fall };
        for (i, p) in self.particles.iter_mut().enumerate() {
            *p += velocity * dt - movement;
            let (half, height) = if i < self.near_from { (HALF_WIDTH, 20.0) } else { (NEAR_HALF_WIDTH, NEAR_HEIGHT) };
            p.x = (p.x + half).rem_euclid(half * 2.0) - half;
            p.y = (p.y + half).rem_euclid(half * 2.0) - half;
            p.z = (p.z + 2.0).rem_euclid(height + 2.0) - 2.0;
        }
        // (a jump of the camera - a new view, a teleport - is no speed)
        let seen = movement / dt.max(0.001);
        velocity - if seen.length() < 80.0 { seen } else { Vec3::ZERO }
    }

    /// Move the particles (camera-relative box) and push them as sprites.
    /// `inside`: the buses near the camera as (origin, heading in degrees, `[boundingbox]`),
    /// the player's first; no drop or flake is drawn within any, so that it does not rain
    /// in a cab (only the player's own had been left dry: riding in another player's bus or
    /// a timetable bus it snowed in the saloon).
    pub fn tick(
        &mut self,
        dt: f32,
        camera: DVec3,
        wind: Vec3,
        scene: &mut Scene,
        inside: &[(DVec3, f64, [f32; 6])],
    ) {
        if self.particles.is_empty() {
            self.camera = Some(camera);
            return;
        }
        let relative = self.advance(dt, camera, wind);
        let streak = relative.normalize_or_zero();
        // the streak: the way the drop goes against the eye while the eye sees it as one
        // (`size` is a quarter of its half length, see corona.wgsl), at least the drop
        let length = (relative.length() * STREAK_TIME).clamp(0.06, 1.4);
        let buses: Vec<(DVec3, f64, [f32; 6])> = inside.iter().filter(|b| (b.0 - camera).length() < 40.0).map(|&(o, h, bb)| (o, h.to_radians(), bb)).collect();
        let in_one = |p: DVec3, (o, h, bb): (DVec3, f64, [f32; 6])| -> bool {
            let d = p - o;
            let (sh, ch) = (h.sin(), h.cos());
            let x = d.x * ch - d.y * sh;
            let y = d.x * sh + d.y * ch;
            let z = d.z;
            // a flat 0.3 m margin left flakes rendering on the inside of the glass near a
            // window sill (the ledge below a side window sits close to the body's outer
            // envelope, closer than the margin covered): widened so the whole cabin,
            // ledges included, sits inside the excluded box.
            (x - bb[3] as f64).abs() < bb[0] as f64 * 0.5 + 0.6
                && (y - bb[4] as f64).abs() < bb[1] as f64 * 0.5 + 0.6
                && (z - bb[5] as f64).abs() < bb[2] as f64 * 0.5 + 0.6
        };
        let in_bus = |p: DVec3| buses.iter().any(|b| in_one(p, *b));
        // (a bigger drop catches more of the light: a drizzle is a haze of faint lines)
        let (size, color, brightness) = if self.kind == 2 {
            (0.06, [1.0, 1.0, 1.0], 0.9)
        } else {
            (length / 8.0, [0.75, 0.8, 0.9], (0.18 + 0.1 * self.drop_mm).min(0.45))
        };
        let mut excluded = 0usize;
        for p in &self.particles {
            let w = camera + p.as_dvec3();
            if in_bus(w) {
                excluded += 1;
                continue;
            }
            scene.coronas.push(Corona {
                position: w,
                size,
                color,
                brightness,
                direction: if self.kind == 2 { Vec3::ZERO } else { streak },
                cone_cos: if self.kind == 2 { -1.0 } else { -2.0 },
                ..Default::default()
            });
        }
        if omsi_cfg::flags::OMSI_DEBUG_RAIN.is_set() {
            log::info!(
                "rain: {} particles, {excluded} inside the buses (boxes {:?})",
                self.particles.len(),
                buses.iter().map(|b| (b.0, b.1.to_degrees(), b.2)).collect::<Vec<_>>()
            );
        }
    }
}

/// The picture the windscreen and window layers wear while it snows.
///
/// Every bus carries the same rain film: a mesh in front of the glass textured with
/// `regen.tga` (running drops) and faded in by `[alphascale] Rain_Window_*_Wetness`, which
/// `rain.osc` fills from `PrecipRate` - and `rain.osc` never asks what is falling, so in a
/// snowstorm the original shows raindrops on every pane, in the cab and along the saloon.
/// OMSI's own way out is the seasonal texture folder (`texture\WinterSnow\`), which no stock
/// vehicle fills in, so openOMSI builds the winter picture itself: crystals settled on
/// the glass, thickest along the rim where a pane collects them, made from the game's own
/// `Texture\snowflake.tga` where it is there and from soft specks of its own where it is not.
pub fn snow_on_glass(root: &std::path::Path) -> omsi_texture::Image {
    // the same grain as the rain film it stands in for (`regen.tga`, 1024 x 1024 with
    // drops a dozen pixels across and a mean alpha of 7 %)
    const SIDE: usize = 512;
    // white where it is clear as well, so that the smaller mip levels stay white specks
    // and do not grey towards the black of transparent texels
    let mut rgba = [255u8, 255, 255, 0].repeat(SIDE * SIDE);
    let flake =
        omsi_texture::decode_file(&omsi_cfg::resolve_path(root, "Texture\\snowflake.tga")).ok();
    let mut rng = 0x9E37_79B9_7F4A_7C15u64;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng >> 40) as f32 / (1u64 << 24) as f32
    };
    // a pane holds more snow at its rim than in the middle, where the airstream and the
    // wipers take it away
    let edge = |x: f32, y: f32| {
        let d = (x - 0.5).abs().max((y - 0.5).abs()) * 2.0;
        0.35 + 0.65 * d * d
    };
    for _ in 0..1800 {
        let (cx, cy) = (rand() * SIDE as f32, rand() * SIDE as f32);
        if rand() > edge(cx / SIDE as f32, cy / SIDE as f32) {
            continue;
        }
        let r = 1.5 + rand() * 3.5;
        let strength = 0.35 + 0.65 * rand();
        let lo = |v: f32| (v - r).max(0.0) as usize;
        let hi = |v: f32| ((v + r) as usize).min(SIDE - 1);
        for y in lo(cy)..=hi(cy) {
            for x in lo(cx)..=hi(cx) {
                let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let d = (dx * dx + dy * dy).sqrt() / r;
                if d > 1.0 {
                    continue;
                }
                // the flake's own picture where the game has one, else a soft round speck
                let a = match &flake {
                    Some(f) => {
                        let u = ((dx / r * 0.5 + 0.5) * (f.width - 1) as f32) as usize;
                        let v = ((dy / r * 0.5 + 0.5) * (f.height - 1) as f32) as usize;
                        let p = (v * f.width as usize + u) * 4;
                        // the stock flake is white on black; its alpha channel, or its
                        // brightness where it has none
                        (f.rgba[p + 3] as f32 / 255.0).max(f.rgba[p] as f32 / 255.0)
                    }
                    None => (1.0 - d * d).powf(1.5),
                };
                let a = (a * strength * 255.0) as u32;
                let p = (y * SIDE + x) * 4;
                rgba[p] = 255;
                rgba[p + 1] = 255;
                rgba[p + 2] = 255;
                rgba[p + 3] = rgba[p + 3].max(a.min(255) as u8);
            }
        }
    }
    omsi_texture::Image {
        width: SIDE as u32,
        height: SIDE as u32,
        rgba,
        has_alpha: true,
    }
}

/// The snow-on-glass picture (`snow_on_glass`) on the GPU, with its mip chain: one level
/// alone, its specks of a pixel or two were point-sampled across a whole windscreen and
/// glittered with every move of the head (#946).
pub fn add_snow_on_glass(
    renderer: &omsi_render::Renderer,
    scene: &mut omsi_render::Scene,
    root: &std::path::Path,
) -> omsi_render::TextureId {
    renderer.add_texture(scene, &snow_on_glass(root), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snow on the glass goes up with its mip levels, and it is white throughout (a
    /// level made smaller stays white, only less opaque).
    #[test]
    fn snow_on_glass_has_mip_levels_and_stays_white() {
        let img = super::snow_on_glass(std::path::Path::new("/nonexistent"));
        assert!(img.rgba.chunks_exact(4).all(|p| p[..3] == [255, 255, 255]));
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::NOOP;
        descriptor.backend_options.noop = wgpu::NoopBackendOptions { enable: true };
        let instance = wgpu::Instance::new(descriptor);
        let renderer = pollster::block_on(omsi_render::Renderer::new_with(
            &instance,
            None,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            omsi_render::RenderOptions { msaa: 1, shadow_size: 1024, ..Default::default() },
        ))
        .expect("noop renderer");
        let mut scene = renderer.new_scene();
        let id = super::add_snow_on_glass(&renderer, &mut scene, std::path::Path::new("/nonexistent"));
        assert_eq!(renderer.texture_levels(&scene, id), Some((512, 512, 10)));
    }

    #[test]
    fn rain_stays_in_the_world_and_streaks_follow_relative_velocity() {
        let mut rain = Rain::new();
        rain.set(1, 1.0, 2);
        rain.particles = vec![Vec3::new(0.0, 0.0, 10.0)];
        rain.camera = Some(DVec3::ZERO);
        let camera = DVec3::new(0.0, 2.0, 0.0);
        rain.near_from = 1;
        let fall = rain.fall;
        let direction = rain.advance(0.1, camera, Vec3::X * 3.0).normalize();
        let world = camera + rain.particles[0].as_dvec3();
        assert!((world - DVec3::new(0.3, 0.0, 10.0 - fall as f64 * 0.1)).length() < 1e-5);
        assert!(direction.abs_diff_eq(Vec3::new(3.0, -20.0, -fall).normalize(), 1e-5));
        rain.advance(0.0, DVec3::splat(10000.0), Vec3::ZERO);
        let p = rain.particles[0];
        assert!(p.is_finite() && p.x.abs() <= HALF_WIDTH && p.y.abs() <= HALF_WIDTH && p.z >= -2.0 && p.z < 20.0);
    }

    #[test]
    fn dry_weather_has_no_particles_and_heavy_rain_keeps_the_budget() {
        let mut rain = Rain::new();
        rain.set(1, 1.0, 2);
        assert_eq!(rain.particles.len(), 6000);
        assert_eq!(rain.near_from, 3000);
        rain.set(1, 0.0, 2);
        assert!(rain.particles.is_empty());
        rain.set(2, 1.0, 2);
        rain.set(0, 1.0, 2);
        assert!(rain.particles.is_empty());
    }

    /// A drizzle falls slower and in smaller drops than a downpour (#1412), as measured.
    #[test]
    fn a_downpour_falls_faster_in_bigger_drops_than_a_drizzle() {
        let drizzle = median_drop_mm(rain_rate_mm_h(0.05));
        let downpour = median_drop_mm(rain_rate_mm_h(1.0));
        assert!(drizzle > 0.5 && drizzle < 1.0, "{drizzle}");
        assert!(downpour > 2.0 && downpour < 2.5, "{downpour}");
        assert!(terminal_speed(drizzle) > 2.0 && terminal_speed(drizzle) < 4.5);
        assert!(terminal_speed(downpour) > 6.5 && terminal_speed(downpour) < 8.0);
        let mut rain = Rain::new();
        rain.set(1, 0.05, 2);
        let slow = rain.fall;
        rain.set(1, 1.0, 2);
        assert!(rain.fall > slow + 2.0);
    }

    #[test]
    fn snow_on_glass_is_mostly_clear_and_thickest_at_the_rim() {
        // no content root here: the procedural specks
        let img = super::snow_on_glass(std::path::Path::new("/nonexistent"));
        assert_eq!((img.width, img.height), (512, 512));
        let alpha_of = |x: usize, y: usize| img.rgba[(y * 512 + x) * 4 + 3] as u32;
        let (mut middle, mut rim, mut n_middle, mut n_rim) = (0u32, 0u32, 0u32, 0u32);
        for y in 0..512 {
            for x in 0..512 {
                let d = ((x as f32 - 255.5).abs()).max((y as f32 - 255.5).abs()) / 255.5;
                if d < 0.35 {
                    middle += alpha_of(x, y);
                    n_middle += 1;
                } else if d > 0.8 {
                    rim += alpha_of(x, y);
                    n_rim += 1;
                }
            }
        }
        let mean = img.rgba.chunks_exact(4).map(|p| p[3] as u32).sum::<u32>() / (512 * 512);
        assert!(mean < 60, "the glass would be opaque (mean alpha {mean})");
        let (a, b) = (rim as f32 / n_rim as f32, middle as f32 / n_middle as f32);
        assert!(
            a > b * 1.3,
            "the rim should carry more snow than the middle ({a:.1} vs {b:.1})"
        );
    }
}
