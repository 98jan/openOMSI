//! The MileZero card over (or under) the navigator: a small bar with the cover, the song, the
//! artist and the time played, which opens into the full card - the DJ, the stream quality,
//! play/pause and a song request - while the mouse is over it. Light or dark like the map.

use super::*;

#[derive(Default)]
pub struct MzState {
    /// How far the card is open (0 small, 1 full), eased.
    k: f32,
    /// The card as drawn this frame, and the buttons of the full card (physical pixels).
    pub rect: Option<[f32; 4]>,
    pub play: Option<[f32; 4]>,
    pub request: Option<[f32; 4]>,
    /// The room it takes beside the navigator, with its gap, for the notices to keep clear.
    pub extent: f32,
    /// Where the card sits inside the information bar (navigator off), when it does.
    pub slot: Option<[f32; 4]>,
}

pub struct MzView {
    pub title: String,
    pub artist: String,
    pub dj: String,
    pub art: Option<std::path::PathBuf>,
    pub elapsed: Option<f32>,
    pub duration: Option<f32>,
    pub playing: bool,
    pub loading: bool,
    pub quality: &'static str,
    /// The request being typed.
    pub typing: Option<String>,
    pub status: Option<String>,
    /// The mouse is over the card (or a request is being typed).
    pub hover: bool,
    /// The light look (the map's `nav_light`).
    pub light: bool,
}

/// The bar's length for a song whose length is not known.
const BAR_SECS: f32 = 210.0;

fn clock(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

impl Ui {
    fn mz_fit(&self, text: &str, px: f32, max: f32) -> String {
        if self.text.width_raw(text, px) <= max {
            return text.to_string();
        }
        let mut t: Vec<char> = text.chars().collect();
        while !t.is_empty() && self.text.width_raw(&format!("{}…", t.iter().collect::<String>()), px) > max {
            t.pop();
        }
        format!("{}…", t.iter().collect::<String>())
    }

    /// The cover, square.
    fn mz_art(&mut self, r: &Renderer, scene: &mut Scene, p: &std::path::Path) -> Option<TextureId> {
        let entry = self.images.entry(p.to_path_buf()).or_insert_with(|| {
            omsi_texture::decode_file(p).ok().map(|img| {
                (r.add_texture(scene, &img, false), img.width, img.height)
            })
        });
        entry.map(|e| e.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn mz_label(&mut self, r: &Renderer, scene: &mut Scene, x_max: f32, text: &str, px: f32, c: [u8; 4], a: f32, x: f32, y: f32) {
        if a < 0.03 {
            return;
        }
        let t = self.mz_fit(text, px, (x_max - x).max(10.0));
        let l = self.text.label_op(r, scene, &t, px.round() as u32, c, a);
        scene.overlays.push((l.tex, [x.round(), y.round(), x.round() + l.w as f32, y.round() + l.h as f32]));
    }

    fn mz_icon(&mut self, r: &Renderer, scene: &mut Scene, name: &str, px: f32, rgba: [u8; 4], x: f32, y: f32) {
        if let Some(t) = self.text.icon(r, scene, name, px.round() as u32, rgba) {
            scene.overlays.push((t, [x.round(), y.round(), x.round() + px.round(), y.round() + px.round()]));
        }
    }

    pub(super) fn draw_milezero(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, s: f32, dt: f32) {
        self.mz.rect = None;
        self.mz.play = None;
        self.mz.request = None;
        self.mz.extent = 0.0;
        let Some(v) = f.milezero else {
            self.mz.k = 0.0;
            return;
        };
        let target = if v.hover { 1.0 } else { 0.0 };
        let k = &mut self.mz.k;
        *k += (target - *k) * (1.0 - (-dt.clamp(0.0, 0.1) * 14.0).exp());
        if (target - *k).abs() < 0.003 {
            *k = target;
        }
        let e = *k * *k * (3.0 - 2.0 * *k);
        let gap = 8.0 * s;
        let slot = if f.nav_rect.is_none() { self.mz.slot } else { None };
        let embedded = slot.is_some();
        let small_w = slot.map_or_else(|| f.nav_rect.map_or(280.0 * s, |a| a[2] - a[0]).max(250.0 * s), |q| q[2] - q[0]);
        let (small_h, big_h) = (slot.map_or(52.0 * s, |q| q[3] - q[1]), 144.0 * s + if v.typing.is_some() || v.status.is_some() { 26.0 * s } else { 0.0 });
        let w = small_w.round();
        let h = (small_h + (big_h - small_h) * e).round();
        let (x0, y0) = match f.nav_rect {
            Some(a) => {
                let left = (a[0] + a[2]) * 0.5 < f.width * 0.5;
                let x = if left { a[0] } else { a[2] - w };
                let y = if a[1] > f.height * 0.5 { a[1] - gap - h } else { a[3] + gap };
                (x, y)
            }
            None => slot.map_or(((f.width - w) * 0.5, 70.0 * s), |q| (q[0], q[1])),
        };
        let x0 = x0.clamp(8.0 * s, (f.width - w - 8.0 * s).max(8.0 * s)).round();
        let y0 = y0.clamp(0.0, (f.height - h).max(0.0)).round();
        let (x1, y1) = (x0 + w, y0 + h);
        self.mz.rect = Some([x0, y0, x1, y1]);
        self.mz.extent = h + gap;
        // the colours of the dark and the light look
        let (bg, text_c, grey, track, accent, key_bg, key_text) = if v.light && !embedded {
            ([250, 250, 252, 250], [28, 28, 34, 0], [104, 104, 116, 0], [0, 0, 0, 34], [40, 96, 235, 255], [226, 227, 234, 255], [28, 28, 34, 255])
        } else {
            ([24, 24, 28, 248], [237, 237, 237, 0], [160, 160, 170, 0], [255, 255, 255, 44], [92, 130, 255, 255], [58, 58, 70, 255], [237, 237, 237, 255])
        };
        // (inside the bar the small card has no plate of its own: the bar is its plate)
        let plate = if embedded { e } else { 1.0 };
        if plate > 0.02 {
            self.text.shadow(r, scene, [x0, y0, x1, y1], 14.0 * s, 20.0 * s, 4.0 * s, ((if v.light { 50.0 } else { 80.0 }) * plate) as u8);
            self.text.rounded(r, scene, [x0, y0, x1, y1], 14.0 * s, [bg[0], bg[1], bg[2], (bg[3] as f32 * plate) as u8]);
        }
        let pad = 12.0 * s;
        let xr = x1 - pad;
        if embedded && e < 0.5 {
            // (a divider, as between the bar's other parts)
            self.text.rounded(r, scene, [x0 - 6.0 * s, y0 + 5.0 * s, x0 - 5.0 * s + 1.0, y1 - 5.0 * s], 0.0, [120, 120, 120, 255]);
        }
        // the cover: 36 px in the small card, 88 in the full one
        let small_art = if embedded { small_h - 8.0 * s } else { 36.0 * s };
        let art = small_art + (72.0 * s - small_art) * e;
        let (ax, ay) = (x0 + if embedded { 4.0 * s } else { pad } + (pad - if embedded { 4.0 * s } else { pad }) * e, y0 + (small_h - small_art) * 0.5 * (1.0 - e) + 12.0 * s * e);
        match v.art.as_deref().and_then(|p| self.mz_art(r, scene, p)) {
            Some(tex) => scene.overlays.push((tex, [ax, ay, ax + art, ay + art])),
            None => self.text.rounded(r, scene, [ax, ay, ax + art, ay + art], 0.0, key_bg),
        }
        let tx = ax + art + (if embedded { 8.0 * s } else { 12.0 * s }) * (1.0 - e) + 12.0 * s * e;
        let title = if v.title.is_empty() { "MileZero".to_string() } else { v.title.clone() };
        let artist = if v.artist.is_empty() { "Live radio".to_string() } else { v.artist.clone() };
        let small = 1.0 - e;
        // the time: played, and of how long where the length is known
        let time = v.elapsed.map(|t| match v.duration {
            Some(d) => format!("{} / {}", clock(t.min(d)), clock(d)),
            None => clock(t),
        });
        let ratio = v.elapsed.map_or(0.0, |t| (t / v.duration.unwrap_or(BAR_SECS).max(1.0)).min(1.0));
        // small card
        let time_w = time.as_ref().map_or(0.0, |t| self.text.width_raw(t, 11.0 * s) + 8.0 * s);
        if embedded {
            let one = format!("{title} · {artist}");
            self.mz_label(r, scene, xr - time_w, &one, 12.0 * s, text_c, small, tx, y0 + (small_h - 15.0 * s) * 0.5);
            if let Some(t) = &time {
                self.mz_label(r, scene, xr + 200.0, t, 11.0 * s, grey, small, xr - time_w + 8.0 * s, y0 + (small_h - 14.0 * s) * 0.5);
            }
        } else {
            self.mz_label(r, scene, xr - time_w, &title, 13.0 * s, text_c, small, tx, y0 + 9.0 * s);
            self.mz_label(r, scene, xr, &artist, 11.0 * s, grey, small, tx, y0 + 27.0 * s);
            if let Some(t) = &time {
                self.mz_label(r, scene, xr + 200.0, t, 11.0 * s, grey, small, xr - time_w + 8.0 * s, y0 + 10.0 * s);
            }
        }
        // full card
        self.mz_label(r, scene, xr, &title, 16.0 * s, text_c, e, tx, y0 + 14.0 * s);
        self.mz_label(r, scene, xr, &artist, 13.0 * s, grey, e, tx, y0 + 36.0 * s);
        if !v.dj.is_empty() {
            self.mz_label(r, scene, xr, &format!("On air: {}", v.dj), 12.0 * s, accent, e, tx, y0 + 58.0 * s);
        }
        self.mz_label(r, scene, xr, &format!("{} · auto quality", v.quality), 11.0 * s, grey, e, tx, y0 + 76.0 * s);
        // the bar along the bottom, with the times above it in the full card
        let (bx0, bx1) = (x0 + pad, xr);
        let by = y1 - 8.0 * s - 4.0 * s * e - (if e > 0.5 { 6.0 * s * e } else { 0.0 });
        let bh = 4.0 * s;
        let bar = if embedded { e } else { 1.0 };
        if bar > 0.3 {
            self.text.rounded(r, scene, [bx0, by, bx1, by + bh], bh * 0.5, track);
        }
        if ratio > 0.0 && bar > 0.3 {
            self.text.rounded(r, scene, [bx0, by, (bx0 + (bx1 - bx0) * ratio).max(bx0 + bh), by + bh], bh * 0.5, accent);
        }
        if let Some(t) = &time {
            let tw = self.text.width_raw(t, 11.0 * s);
            self.mz_label(r, scene, xr + 200.0, t, 11.0 * s, grey, e, xr - tw, y0 + 76.0 * s);
        }
        // buttons (only once the card is nearly open, so a click on the small one does nothing)
        if e > 0.9 {
            let bh = 30.0 * s;
            let row_y = y0 + 92.0 * s;
            let bw = (w - pad * 2.0 - 8.0 * s) * 0.5;
            let play = [x0 + pad, row_y, x0 + pad + bw, row_y + bh];
            let req = [play[2] + 8.0 * s, row_y, xr, row_y + bh];
            let (name, icon, tint) = if v.loading { ("Loading", "radio", [110, 120, 160, 255]) } else if v.playing { ("Pause", "pause", accent) } else { ("Play", "play_arrow", accent) };
            self.text.rounded(r, scene, play, bh * 0.5, tint);
            self.text.rounded(r, scene, req, bh * 0.5, key_bg);
            for (rect, text, icon, color) in [(play, name, icon, [255, 255, 255, 255]), (req, "Request", "radio", key_text)] {
                let ip = 16.0 * s;
                let l = self.text.label_op(r, scene, text, (12.0 * s).round() as u32, [color[0], color[1], color[2], 0], 1.0);
                let total = ip + 6.0 * s + l.w as f32;
                let lx = ((rect[0] + rect[2] - total) * 0.5).round();
                self.mz_icon(r, scene, icon, ip, color, lx, (rect[1] + rect[3] - ip) * 0.5);
                let tx = lx + ip + 6.0 * s;
                let ty = ((rect[1] + rect[3] - l.h as f32) * 0.5).round();
                scene.overlays.push((l.tex, [tx, ty, tx + l.w as f32, ty + l.h as f32]));
            }
            self.mz.play = Some(play);
            self.mz.request = Some(req);
            // the request being typed, or what became of the last one
            let line = match (&v.typing, &v.status) {
                (Some(t), _) => Some((format!("{t}|"), text_c, "close")),
                (None, Some(m)) => Some((m.clone(), [60, 160, 100, 0], "")),
                _ => None,
            };
            if let Some((text, c, _)) = line {
                let ly = y0 + 130.0 * s;
                let chip = [x0 + pad, ly - 3.0 * s, xr, ly + 19.0 * s];
                self.text.rounded(r, scene, chip, 8.0 * s, key_bg);
                let shown = if v.typing.is_some() && text.len() <= 1 { "Type a song and press Enter (Esc cancels)".to_string() } else { text };
                let c = if v.typing.is_some() && shown.starts_with("Type a song") { grey } else { c };
                self.mz_label(r, scene, xr - 6.0 * s, &shown, 12.0 * s, c, 1.0, x0 + pad + 8.0 * s, ly);
            }
        }
    }
}
