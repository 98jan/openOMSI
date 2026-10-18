//! The MileZero card over (or under) the navigator: a small bar with the cover, the song, the
//! artist and the seconds played, which opens into the full card - the station's name for
//! the DJ, the stream quality, play/pause and a song request - while the mouse is over it.

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
}

pub struct MzView {
    pub title: String,
    pub artist: String,
    pub dj: String,
    pub art: Option<std::path::PathBuf>,
    pub elapsed: Option<f32>,
    pub playing: bool,
    pub loading: bool,
    pub quality: &'static str,
    /// The request being typed.
    pub typing: Option<String>,
    pub status: Option<String>,
    /// The mouse is over the card (or a request is being typed).
    pub hover: bool,
}

/// A song is of no known length (the station does not say): the bar fills over this.
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

    fn mz_art(&mut self, r: &Renderer, scene: &mut Scene, p: &std::path::Path) -> Option<TextureId> {
        let entry = self.images.entry(p.to_path_buf()).or_insert_with(|| {
            omsi_texture::decode_file(p).ok().map(|img| {
                let (iw, ih) = (img.width, img.height);
                (r.add_texture(scene, &img, false), iw, ih)
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
        let (small_w, small_h, big_w) = ((f.nav_rect.map_or(280.0 * s, |a| a[2] - a[0])).clamp(250.0 * s, 360.0 * s), 52.0 * s, 340.0 * s);
        let w = (small_w + (big_w.max(small_w) - small_w) * e).round();
        let h = (small_h + (190.0 * s - small_h) * e).round();
        let (x0, y0) = match f.nav_rect {
            Some(a) => {
                let left = (a[0] + a[2]) * 0.5 < f.width * 0.5;
                let x = if left { a[0] } else { a[2] - w };
                let y = if a[1] > f.height * 0.5 { a[1] - gap - h } else { a[3] + gap };
                (x, y)
            }
            None => ((f.width - w) * 0.5, 70.0 * s),
        };
        let x0 = x0.clamp(8.0 * s, (f.width - w - 8.0 * s).max(8.0 * s)).round();
        let y0 = y0.clamp(0.0, (f.height - h).max(0.0)).round();
        let (x1, y1) = (x0 + w, y0 + h);
        self.mz.rect = Some([x0, y0, x1, y1]);
        self.mz.extent = h + gap;
        self.text.shadow(r, scene, [x0, y0, x1, y1], 12.0 * s, 20.0 * s, 4.0 * s, 80);
        self.text.rounded(r, scene, [x0, y0, x1, y1], 12.0 * s, [24, 24, 28, 248]);
        let white = [237, 237, 237, 0];
        let grey = [160, 160, 170, 0];
        let ratio = v.elapsed.map_or(0.0, |t| (t / BAR_SECS).min(0.98));
        let pad = 10.0 * s;
        // the cover: 34 px in the small card, 92 px in the full one
        let art = 34.0 * s + (92.0 * s - 34.0 * s) * e;
        let (ax, ay) = (x0 + pad, y0 + (small_h - 34.0 * s) * 0.5 * (1.0 - e) + 12.0 * s * e);
        match v.art.as_deref().and_then(|p| self.mz_art(r, scene, p)) {
            Some(tex) => scene.overlays.push((tex, [ax, ay, ax + art, ay + art])),
            None => self.text.rounded(r, scene, [ax, ay, ax + art, ay + art], 6.0 * s, [52, 52, 62, 255]),
        }
        let tx = ax + art + 10.0 * s;
        let small = 1.0 - e;
        let xr = x1 - pad;
        let title = if v.title.is_empty() { "MileZero".to_string() } else { v.title.clone() };
        let artist = if v.artist.is_empty() { "Live radio".to_string() } else { v.artist.clone() };
        // small card: the song, the artist, the seconds played
        let time_w = 38.0 * s;
        self.mz_label(r, scene, xr, &title, 13.0 * s, white, small, tx, y0 + 8.0 * s);
        self.mz_label(r, scene, xr, &artist, 11.0 * s, grey, small, tx, y0 + 26.0 * s);
        if let Some(t) = v.elapsed {
            self.mz_label(r, scene, xr, &clock(t), 11.0 * s, grey, small, x1 - pad - time_w + 8.0 * s, y0 + 8.0 * s);
        }
        // full card: everything the station says
        self.mz_label(r, scene, xr, &title, 16.0 * s, white, e, tx, y0 + 14.0 * s);
        self.mz_label(r, scene, xr, &artist, 13.0 * s, grey, e, tx, y0 + 38.0 * s);
        if !v.dj.is_empty() {
            self.mz_label(r, scene, xr, &format!("On air: {}", v.dj), 12.0 * s, [120, 160, 255, 0], e, tx, y0 + 62.0 * s);
        }
        self.mz_label(r, scene, xr, &format!("{} (auto quality)", v.quality), 11.0 * s, grey, e, tx, y0 + 82.0 * s);
        // the bar along the bottom of both
        let by = y1 - 12.0 * s;
        let (bx0, bx1) = (x0 + pad, x1 - pad);
        self.text.rounded(r, scene, [bx0, by, bx1, by + 3.0 * s], 1.5 * s, [255, 255, 255, 40]);
        if ratio > 0.0 {
            self.text.rounded(r, scene, [bx0, by, bx0 + (bx1 - bx0) * ratio, by + 3.0 * s], 1.5 * s, [92, 130, 255, 255]);
        }
        if e > 0.5 {
            if let Some(t) = v.elapsed {
                self.mz_label(r, scene, xr, &clock(t), 11.0 * s, grey, e, bx0, by - 18.0 * s);
            }
        }
        // buttons (only once the card is nearly open, so a click on the small one does nothing)
        if e > 0.9 {
            let bh = 28.0 * s;
            let row_y = y0 + 108.0 * s;
            let play = [x0 + pad, row_y, x0 + pad + 104.0 * s, row_y + bh];
            let req = [play[2] + 8.0 * s, row_y, play[2] + 8.0 * s + 104.0 * s, row_y + bh];
            let (name, tint) = if v.loading { ("Loading ...", [70, 70, 90, 255]) } else if v.playing { ("Pause", [60, 90, 200, 255]) } else { ("Play", [60, 110, 230, 255]) };
            self.text.rounded(r, scene, play, 8.0 * s, tint);
            self.text.rounded(r, scene, req, 8.0 * s, [58, 58, 70, 255]);
            for (rect, text) in [(play, name), (req, "Request")] {
                let l = self.text.label_op(r, scene, text, (12.0 * s).round() as u32, white, 1.0);
                let (lx, ly) = ((rect[0] + rect[2] - l.w as f32) * 0.5, (rect[1] + rect[3] - l.h as f32) * 0.5);
                scene.overlays.push((l.tex, [lx.round(), ly.round(), lx.round() + l.w as f32, ly.round() + l.h as f32]));
            }
            self.mz.play = Some(play);
            self.mz.request = Some(req);
            // the request being typed, or what became of the last one
            let line_y = y0 + 142.0 * s;
            let line = match (&v.typing, &v.status) {
                (Some(t), _) => Some((format!("Request: {t}|   (Enter sends, Esc cancels)"), [237, 237, 237, 0])),
                (None, Some(s)) => Some((s.clone(), [150, 200, 150, 0])),
                _ => None,
            };
            if let Some((text, c)) = line {
                self.mz_label(r, scene, xr, &text, 12.0 * s, c, 1.0, x0 + pad, line_y);
            }
        }
    }
}
