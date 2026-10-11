//! The game's own interface, drawn over the picture in Roboto with a dark outline (the
//! HUD's `.oft` fonts are OMSI's and stay for the time and speed): the chat of a LAN
//! session, the name of what the cursor points at next to the cursor, and the other
//! players' name tags above their buses.
//!
//! Every text is rendered once into a small texture and kept while it is shown; the
//! overlays are rectangles in physical pixels (`Scene::overlays`).

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};
use omsi_render::{Renderer, Scene, TextureId};

mod menu;
mod text;

pub use text::TextCache;
use menu::*;
use text::*;

// ---------------------------------------------------------------------------------------
// the chat

/// How many rows the chat shows (a long line takes several), and how many lines it keeps to
/// scroll back.
const CHAT_SHOWN: usize = 8;
pub const CHAT_KEEP: usize = 200;

/// A notification from the server (`notify`): a card over the navigator that goes by itself.
#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    pub kind: NoticeKind,
    pub text: String,
    /// Seconds it still shows, of `total`.
    pub left: f32,
    pub total: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoticeKind {
    Info,
    Warn,
    Alert,
    /// A new openOMSI version (`update_watch`).
    Update,
}

/// How many notifications show at once (the newest; an older one makes room).
pub const NOTICES_SHOWN: usize = 3;

impl Notice {
    /// `notify`'s argument: `<id> <seconds> <info|warn|alert> <text>` → the id and the notice
    /// (seconds held to 2 .. 30; an unknown kind is information).
    pub fn parse(arg: &str) -> Option<(String, Notice)> {
        let mut it = arg.trim().splitn(4, ' ');
        let id = it.next().filter(|s| !s.is_empty())?.to_string();
        let secs: f32 = it.next()?.parse().ok().filter(|s: &f32| s.is_finite())?;
        let kind = match it.next()? {
            "warn" => NoticeKind::Warn,
            "alert" => NoticeKind::Alert,
            _ => NoticeKind::Info,
        };
        let text = it.next().map(str::trim).filter(|t| !t.is_empty())?.to_string();
        let total = secs.clamp(2.0, 30.0);
        Some((id, Notice { kind, text, left: total, total }))
    }

    /// How opaque it is: fading in for a quarter second, out for its last half second.
    pub fn alpha(&self) -> f32 {
        (self.left / 0.5).min((self.total - self.left) / 0.25).clamp(0.0, 1.0)
    }
}

/// A new notice in the list, the oldest going when there are more than `NOTICES_SHOWN`.
pub fn push_notice(list: &mut Vec<Notice>, n: Notice) {
    list.push(n);
    while list.len() > NOTICES_SHOWN {
        list.remove(0);
    }
}

/// What the chat widget needs from the session each frame.
pub struct ChatView<'a> {
    /// Every line, oldest first ("Name: text" or "* notice").
    pub lines: &'a [String],
    /// The line being typed (the input box is open).
    pub typing: Option<&'a str>,
    /// Why the last line was not sent.
    pub error: Option<&'a str>,
}

/// The chat's own state: shown or hidden (V), the scroll position and whether the cursor is
/// over it.
#[derive(Default)]
pub struct ChatWidget {
    pub hidden: bool,
    /// Lines scrolled back from the newest.
    pub scroll: usize,
    /// The chat's box on the screen (physical pixels) as drawn last: hovering over it shows
    /// the input box, a click there opens it.
    pub rect: [f32; 4],
    pub hovered: bool,
    pub caret_t: f32,
}

impl ChatWidget {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.rect[0] && x <= self.rect[2] && y >= self.rect[1] && y <= self.rect[3]
    }

    /// The mouse wheel over the chat (or while typing) scrolls the history.
    pub fn wheel(&mut self, lines: usize, amount: f32) {
        let max = lines.saturating_sub(CHAT_SHOWN);
        let s = self.scroll as i32 + amount.round() as i32;
        self.scroll = s.clamp(0, max as i32) as usize;
    }
}

/// How much larger than designed the interface is drawn, on top of the screen's scale
/// `dpi`: on a window taller than 1080 logical pixels as much as it is taller (up to twice),
/// times the player's `size` (`Settings::ui_scale`). Laid out for 1080p, the texts were half
/// their size on a 4K screen at 100 % display scaling; a window of 1080 lines or fewer is
/// drawn as it always was. With `window` off (`Settings::ui_scale_window`) it does not grow
/// with the window at all.
pub fn size_factor(height_px: f32, dpi: f32, size: f32, window: bool) -> f32 {
    let grown = if window { (height_px / dpi.max(0.5) / 1080.0).clamp(1.0, 2.0) } else { 1.0 };
    grown * size
}

/// How strongly the interface's backgrounds are drawn for the opacity setting
/// (`Settings::ui_opacity`): 1 at its default of 85 %, as designed; lower, the picture shows
/// through them (never less than 0.3), higher a little darker. The texts stay solid.
pub fn backdrop(opacity: f32) -> f32 {
    (opacity / 0.85).clamp(0.3, 1.3)
}

/// Which menu is open: its layout follows from it.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MenuKind {
    /// The game menu itself.
    #[default]
    Game,
    /// A settings window (options, vehicle, world): a sidebar of pages and rows with switches,
    /// sliders and buttons. A row's label is `name\u{1f}kind\u{1f}value\u{1f}description\u{1f}fraction`
    /// (kind: `s` switch, `v` slider, `c` stepper, `o` opens a list, `a` button, `i` information).
    Options,
    /// The lines of the map's timetable.
    Lines,
    /// One line's tours.
    Tours,
    /// Any other list (drivers, fleet numbers, destinations, liveries ...).
    List,
}

/// The timetable beside a list of lines or tours: a title, a line of facts and rows of
/// (what, time).
pub struct Preview {
    pub title: String,
    pub meta: String,
    pub rows: Vec<(String, String)>,
    /// The row chosen, when the rows can be chosen (the stops to start a tour from).
    pub chosen: Option<usize>,
    /// The button under the rows, when there is one.
    pub button: Option<String>,
    /// The time of the timetable the rows are of, with buttons to put it ahead (the trip of
    /// a tour to take on later), when it can be set.
    pub time: Option<String>,
}


/// A drop-down open over a row of a settings window: the entries, the one the keyboard is
/// on, the first one shown and the one in force now.
pub struct DropdownView<'a> {
    pub row: usize,
    pub items: Vec<&'a str>,
    pub sel: usize,
    pub top: usize,
    pub current: Option<usize>,
}

/// Everything the interface draws in a frame.
pub struct Frame<'a> {
    /// Physical pixels per logical one.
    pub scale: f32,
    /// How much larger than designed (`size_factor`): everything below is drawn this much
    /// larger on top of `scale`.
    pub ui_scale: f32,
    /// How strongly the backgrounds are drawn (`backdrop`): the menu's and the timetable's
    /// panels, the chat's box, the plates under the notes, the tooltip and the frame rate.
    pub opacity: f32,
    pub width: f32,
    pub height: f32,
    pub cursor: (f32, f32),
    /// The mouse steering's cross, while the system cursor is hidden and held (where the
    /// crosshair cursor stood before): drawn here instead.
    pub steer_cross: Option<(f32, f32)>,
    /// An OpenXR headset is drawing this frame.
    pub vr: bool,
    /// The name of what the cursor points at (a switch, a part), shown next to it.
    pub tooltip: Option<String>,
    /// The chat, when a LAN session runs and the chat is not switched off.
    pub chat: Option<ChatView<'a>>,
    /// The chat's own size on top of `ui_scale` (`Settings::chat_size`).
    pub chat_size: f32,
    /// What the driver has to act on (why the bus does not move, a passenger's wish, the
    /// change due, a service done), top left.
    pub notes: &'a [String],
    /// The frame rate, top right (the `show_fps` setting).
    pub fps: Option<f32>,
    /// The game stands paused.
    pub paused: bool,
    /// The game menu is open, with this line chosen (labels from `GAME_MENU`).
    pub menu: Option<(usize, &'a [(&'a str, &'a str)])>,
    /// The first line shown when a finger scrolled the menu (`App::menu_top`).
    pub menu_top: Option<f32>,
    /// Ids of the game menu's lines that are greyed out and cannot be chosen (the timetable
    /// without an active route).
    pub menu_disabled: &'a [&'a str],
    /// The timetable window: its title and per stop (name, time, 0 served / 1 next / 2 ahead).
    pub timetable: Option<(String, Vec<(String, String, u8)>)>,
    /// The information bar along the top: its parts between [`INFO_SEP`]s.
    pub info: Option<String>,
    /// The room the information bar has along the top: from x to x, from y down (on a
    /// touch screen the gap between its buttons); none: the window's width.
    pub info_room: Option<[f32; 3]>,
    /// A tutorial page: title, text, picture, page number and count.
    pub tutorial: Option<(&'a str, &'a str, Option<&'a std::path::Path>, usize, usize)>,
    /// Name tags: a screen position (the point above a bus), the name and a second line.
    pub tags: Vec<((f32, f32), String, String, f32)>,
    /// The server's notifications, oldest first.
    pub notices: &'a [Notice],
    /// Where the navigator is on the screen (the notifications stand over it, or under it
    /// in a top corner); none: they go to the top middle.
    pub notice_anchor: Option<[f32; 4]>,
    /// What kind of menu the lines belong to.
    pub menu_kind: MenuKind,
    /// The open list's title and the small line above it (the line a tour list is of).
    pub menu_head: Option<(String, String)>,
    /// The timetable of the chosen line or tour, beside the list.
    pub menu_preview: Option<Preview>,
    /// The first stop shown of the timetable beside the tours, when the wheel has scrolled it
    /// (`None`: the stop chosen is kept in view).
    pub pane_first: Option<usize>,
    /// The pages of an open settings window (their titles) and the one shown.
    pub menu_tabs: Option<(Vec<String>, usize)>,
    /// The keyboard chose the menu's line last: that line is shown lit (else only the one
    /// under the mouse is).
    pub menu_kbd: bool,
    /// The drop-down open over a row of the settings window.
    pub dropdown: Option<DropdownView<'a>>,
}

pub struct Ui {
    origin_x: f32,
    pub text: TextCache,
    pub chat: ChatWidget,
    /// Where the game menu's lines were drawn this frame (physical pixels), for the mouse.
    pub menu_rects: Vec<[f32; 4]>,
    /// Per line of `menu_rects`, where the arrows round its value are (a list's setting,
    /// `game_lists::ADJUST`): `[from, to, plus]` - a click from `from` to `to` steps it
    /// down, one right of `plus` up.
    pub menu_arrows: Vec<Option<[f32; 3]>>,
    pub menu_scroll_thumb: Option<[f32; 4]>,
    pub menu_scroll_track: Option<[f32; 4]>,
    /// Where the controls of the settings rows were drawn (a slider's track, a stepper), one
    /// entry per line in `menu_rects`: a click there sets the value.
    pub menu_ctl: Vec<Option<[f32; 4]>>,
    /// The sidebar of a settings window: one box per page, the way back last.
    pub menu_side: Vec<[f32; 4]>,
    /// The rows of the timetable beside a line's tours (the stops to start from), the stop the
    /// first of them is, and the button that starts the trip.
    pub menu_pane: Vec<[f32; 4]>,
    pub menu_pane_start: usize,
    pub menu_pane_go: Option<[f32; 4]>,
    /// The whole timetable pane beside the tours: the wheel over it scrolls its stops.
    pub menu_pane_box: Option<[f32; 4]>,
    /// Its scroll bar when it has more stops than it shows: the track, the thumb (widened
    /// to be hit), how many stops there are and how many it shows - for the mouse to drag.
    pub menu_pane_scroll: Option<([f32; 4], [f32; 4], usize, usize)>,
    /// The two arrows beside the time of a tour: the trip before, the next one.
    pub menu_time: Vec<[f32; 4]>,
    /// The colours and positions of the menu's parts that ease to their new state (a line's
    /// light, a switch's knob ...), by what they belong to.
    anim: std::collections::HashMap<u64, f32>,
    /// Seconds since the last frame, for `ease`.
    anim_dt: f32,
    /// Overlay entries belonging to the game menu.
    pub menu_overlay_range: std::ops::Range<usize>,
    /// The pointer texture, positioned separately for each headset eye.
    pub vr_cursor_overlay: Option<usize>,
    pub vr_tooltip_overlay: Option<usize>,
    /// The first line of the menu shown (a long menu scrolls: `menu_rects[k]` is line
    /// `menu_start + k`).
    pub menu_start: usize,
    /// How many lines the menu shows at once, and how high one is (physical pixels): a
    /// finger's drag is turned into lines with it.
    pub menu_rows: usize,
    pub menu_row_h: f32,
    /// The entries of the drop-down shown (their rects), the first of them, and how many fit.
    pub dd_rects: Vec<[f32; 4]>,
    pub dd_top: usize,
    pub dd_rows: usize,
    /// The drop-down's scroll bar when it has more entries than it shows: its track and
    /// its thumb (widened to be hit), for the mouse to drag.
    pub dd_scroll: Option<([f32; 4], [f32; 4])>,
    /// Pictures shown in the interface (a tutorial page's), by file.
    images: hashbrown::HashMap<std::path::PathBuf, Option<(TextureId, u32, u32)>>,
    /// Where the information bar was drawn (within the panel, before `origin_x`), for the
    /// navigator to keep out of its way.
    pub info_rect: Option<[f32; 4]>,
}

/// Between the information bar's parts.
pub(crate) const INFO_SEP: &str = "   |   ";

/// The information bar's parts in rows no wider than `room` (as `width` measures a text),
/// each as many parts as fit: in one line it ran off both sides of a narrow window, and on a
/// phone it lay under the buttons along the top (#1164).
pub(crate) fn info_rows(line: &str, room: f32, width: impl Fn(&str) -> f32) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    for part in line.split(INFO_SEP) {
        match rows.last_mut() {
            Some(row) if width(&format!("{row}{INFO_SEP}{part}")) <= room => {
                row.push_str(INFO_SEP);
                row.push_str(part);
            }
            _ => rows.push(part.to_string()),
        }
    }
    rows
}

pub(crate) fn shift_overlays(scene: &mut Scene, start: usize, x: f32) {
    for (_, rect) in &mut scene.overlays[start..] {
        rect[0] += x;
        rect[2] += x;
    }
}

impl Ui {
    /// Lay out at panel resolution, then move both pixels and mouse targets to
    /// that panel's position in the spanning window.
    pub fn draw_at(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, dt: f32, origin_x: f32) {
        self.shift_hitboxes(-self.origin_x);
        let start = scene.overlays.len();
        self.draw(r, scene, f, dt);
        shift_overlays(scene, start, origin_x);
        self.shift_hitboxes(origin_x);
        self.origin_x = origin_x;
    }

    fn shift_hitboxes(&mut self, x: f32) {
        for rect in self
            .menu_rects
            .iter_mut()
            .chain(self.menu_side.iter_mut())
            .chain(self.menu_pane.iter_mut())
            .chain(self.menu_time.iter_mut())
            .chain(self.dd_rects.iter_mut())
            .chain(self.menu_ctl.iter_mut().flatten())
            .chain(self.menu_scroll_thumb.iter_mut())
            .chain(self.menu_scroll_track.iter_mut())
            .chain(self.menu_pane_go.iter_mut())
            .chain(self.menu_pane_box.iter_mut())
        {
            rect[0] += x;
            rect[2] += x;
        }
        for arrows in self.menu_arrows.iter_mut().flatten() {
            for at in arrows {
                *at += x;
            }
        }
        if let Some((track, thumb)) = &mut self.dd_scroll {
            for rect in [track, thumb] {
                rect[0] += x;
                rect[2] += x;
            }
        }
        if let Some((track, thumb, _, _)) = &mut self.menu_pane_scroll {
            for rect in [track, thumb] {
                rect[0] += x;
                rect[2] += x;
            }
        }
        self.chat.rect[0] += x;
        self.chat.rect[2] += x;
    }
    pub fn new() -> Option<Ui> {
        Some(Ui { origin_x: 0.0, text: TextCache::new()?, chat: ChatWidget::default(), menu_rects: Vec::new(), menu_arrows: Vec::new(), menu_scroll_thumb: None, menu_scroll_track: None, menu_ctl: Vec::new(), dd_rects: Vec::new(), dd_top: 0, dd_rows: 8, dd_scroll: None, menu_side: Vec::new(), menu_pane: Vec::new(), menu_pane_start: 0, menu_pane_go: None, menu_pane_box: None, menu_pane_scroll: None, menu_time: Vec::new(), anim: Default::default(), anim_dt: 0.0, menu_overlay_range: 0..0, vr_cursor_overlay: None, vr_tooltip_overlay: None, menu_start: 0, menu_rows: 0, menu_row_h: 1.0, images: Default::default(), info_rect: None })
    }

    /// Draw the frame's interface: its overlays go after the HUD's in `scene.overlays`.
    pub fn draw(&mut self, r: &Renderer, scene: &mut Scene, f: &Frame, dt: f32) {
        let s = f.scale.max(0.5) * f.ui_scale;
        self.text.backdrop = f.opacity;
        // --- name tags above the other players' buses
        for ((x, y), name, sub, alpha) in &f.tags {
            let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
            let l = self.text.label(r, scene, name, (19.0 * s) as u32, [255, 255, 255, 220]);
            let x0 = x - l.w as f32 * 0.5;
            let y0 = y - l.h as f32;
            if a > 0 {
                scene.overlays.push((l.tex, [x0, y0, x0 + l.w as f32, y0 + l.h as f32]));
                if !sub.is_empty() {
                    let m = self.text.label(r, scene, sub, (12.0 * s) as u32, [210, 225, 255, 200]);
                    let mx = x - m.w as f32 * 0.5;
                    scene.overlays.push((m.tex, [mx, y0 + l.h as f32 - 3.0 * s, mx + m.w as f32, y0 + l.h as f32 - 3.0 * s + m.h as f32]));
                }
            }
        }
        // the top of the windows in the corners: the notes on the left start on the line of
        // the timetable and a tutorial page on the right (14 px down, they stood higher)
        let corner_top = 60.0 * s;
        // --- notes, top left: white on a dark outline, one line each, on a dim plate (the
        // outline alone left a note hard to read over a bright sky)
        let notes_bottom = {
            let px = (16.0 * s) as u32;
            let x0 = 16.0 * s;
            // (below the on-screen buttons of a phone, which keep their size when the
            // interface is made smaller)
            let mut y = if crate::platform::touch_controls() { (80.0 * f.scale.max(0.5) * f.ui_scale.max(1.0)).max(corner_top) } else { corner_top };
            for n in f.notes.iter().filter(|n| !n.trim().is_empty()).take(8) {
                let text = omsi_ui::tr(n);
                let text = clip_to(&self.text, &text, px as f32, f.width * 0.6);
                let l = self.text.label(r, scene, &text, px, [255, 255, 255, 235]);
                let plate = self.text.plate(r, scene, 7);
                scene.overlays.push((plate, [x0 - 5.0 * s, y, x0 + l.w as f32 + 5.0 * s, y + l.h as f32]));
                scene.overlays.push((l.tex, [x0, y, x0 + l.w as f32, y + l.h as f32]));
                y += l.h as f32 + 2.0 * s;
            }
            y
        };
        // --- the chat, top left under the notes, as Roblox has it (from the fourth note
        // on they ran into it)
        if let Some(c) = f.chat.as_ref().filter(|_| !self.chat.hidden) {
            // (its own size on top of the interface's: Ctrl + the wheel over it)
            let s = s * f.chat_size.clamp(0.5, 3.0);
            let px = (17.0 * s) as u32;
            let lh = px as f32 * 1.35;
            let x0 = 14.0 * s;
            let y0 = (96.0 * s).max(notes_bottom + 10.0 * s);
            let width = (460.0 * s).min(f.width * 0.6);
            let open = c.typing.is_some();
            let n = c.lines.len();
            let end = n.saturating_sub(if open || self.chat.hovered { self.chat.scroll } else { 0 });
            let box_h = lh * CHAT_SHOWN as f32 + lh * 1.6;
            self.chat.rect = [x0 - 6.0 * s, y0 - 6.0 * s, x0 + width, y0 + box_h];
            self.chat.hovered = self.chat.contains(f.cursor.0, f.cursor.1);
            let show_box = open || self.chat.hovered;
            // the lines, newest at the bottom of the history area; a line too long for the box
            // goes on in rows under it, indented (the oldest line shown may lose its first rows)
            let indent = 14.0 * s;
            let mut rows: Vec<(f32, String, [u8; 4])> = Vec::new();
            {
                let tc = &self.text;
                let measure = |t: &str| tc.width_raw(t, px as f32);
                let mut i = end;
                while i > 0 && rows.len() < CHAT_SHOWN {
                    i -= 1;
                    let line = &c.lines[i];
                    let color = if line.starts_with("* ") { [255, 226, 140, 230] } else { [255, 255, 255, 230] };
                    for (k, t) in wrap_rows(line, width, indent, &measure).into_iter().enumerate().rev() {
                        if rows.len() == CHAT_SHOWN {
                            break;
                        }
                        rows.push((if k == 0 { 0.0 } else { indent }, t, color));
                    }
                }
                rows.reverse();
            }
            let mut y = y0 + lh * (CHAT_SHOWN - rows.len()) as f32;
            for (dx, text, color) in rows {
                let l = self.text.label(r, scene, &text, px, color);
                scene.overlays.push((l.tex, [x0 + dx, y, x0 + dx + l.w as f32, y + l.h as f32]));
                y += lh;
            }
            if show_box {
                // the input box: shown when the cursor is over the chat or it is typed into
                let by = y0 + lh * CHAT_SHOWN as f32 + lh * 0.2;
                let bh = lh * 1.25;
                let plate = self.text.plate(r, scene, 0);
                scene.overlays.push((plate, [x0 - 4.0 * s, by, x0 + width, by + bh]));
                self.chat.caret_t += dt;
                let caret = if open && (self.chat.caret_t % 1.0) < 0.55 { "|" } else { "" };
                let (text, color) = match c.typing {
                    Some(t) => (format!("{t}{caret}"), [255, 255, 255, 240]),
                    None => ("Click here or press / to chat".to_string(), [190, 190, 190, 200]),
                };
                let text = clip_left(&self.text, &text, px as f32, width - 10.0 * s);
                let l = self.text.label(r, scene, &text, px, color);
                let ty = by + (bh - l.h as f32) * 0.5;
                scene.overlays.push((l.tex, [x0 + 2.0 * s, ty, x0 + 2.0 * s + l.w as f32, ty + l.h as f32]));
                if self.chat.scroll > 0 && (open || self.chat.hovered) {
                    // (the words translated apart from the number: whole, the text was no key)
                    let m = self.text.label(r, scene, &format!("{} {}", self.chat.scroll, omsi_ui::tr("newer below")), (11.0 * s) as u32, [200, 200, 200, 200]);
                    scene.overlays.push((m.tex, [x0 + width - m.w as f32, by - m.h as f32, x0 + width, by]));
                }
            }
            if let Some(e) = c.error {
                let l = self.text.label(r, scene, &format!("{}: {}", omsi_ui::tr("Not sent"), omsi_ui::tr(e)), (12.0 * s) as u32, [255, 150, 150, 220]);
                let ey = y0 + box_h;
                scene.overlays.push((l.tex, [x0, ey, x0 + l.w as f32, ey + l.h as f32]));
            }
        } else {
            self.chat.hovered = false;
            self.chat.rect = [0.0; 4];
        }
        // --- the server's notifications: cards over the navigator (under it when it is in a
        // top corner; at the top middle without it), the newest nearest to it
        if !f.notices.is_empty() {
            let pad = 10.0 * s;
            let stripe = 4.0 * s;
            let gap = 6.0 * s;
            let (x0, w, mut y, up) = match f.notice_anchor {
                Some(a) => {
                    let w = (a[2] - a[0]).max(300.0 * s).min(f.width - 16.0 * s);
                    let x0 = if a[0] + w > f.width { f.width - w - 8.0 * s } else { a[0] }.max(8.0 * s);
                    if a[1] > f.height * 0.5 { (x0, w, a[1] - gap, true) } else { (x0, w, a[3] + gap, false) }
                }
                None => {
                    let w = (420.0 * s).min(f.width - 32.0 * s);
                    ((f.width - w) * 0.5, w, 70.0 * s, false)
                }
            };
            let title_px = (13.0 * s) as u32;
            let px = (16.0 * s) as u32;
            let lh = px as f32 * 1.3;
            for n in f.notices.iter().rev() {
                // (the text steps in quarters as it fades: a label is made for each colour)
                let a = (n.alpha() * 4.0).ceil() / 4.0;
                if a <= 0.0 {
                    continue;
                }
                let (accent, title) = match n.kind {
                    NoticeKind::Info => ([90, 160, 255], "Server"),
                    NoticeKind::Warn => ([240, 170, 50], "Warning"),
                    NoticeKind::Alert => ([235, 80, 70], "Alert"),
                    NoticeKind::Update => ([46, 160, 67], "Update"),
                };
                let rows = {
                    let tc = &self.text;
                    wrap_rows(&n.text, w - stripe - pad * 2.0, 0.0, &|t: &str| tc.width_raw(t, px as f32))
                };
                // (five rows at the most; a longer text says it goes on)
                let mut rows: Vec<String> = rows.iter().map(|r| r.to_string()).collect();
                if rows.len() > 5 {
                    rows.truncate(5);
                    rows[4].push('…');
                }
                let th = title_px as f32 * 1.35;
                let h = pad + th + lh * rows.len() as f32 + pad * 0.7;
                let top = if up { y - h } else { y };
                // (a stack that would leave the window ends there)
                if top < 0.0 || top + h > f.height {
                    break;
                }
                let bg = self.text.solid(r, scene, [14, 16, 20, (225.0 * a * self.text.backdrop) as u8]);
                scene.overlays.push((bg, [x0, top, x0 + w, top + h]));
                let bar = self.text.solid(r, scene, [accent[0], accent[1], accent[2], (255.0 * a) as u8]);
                scene.overlays.push((bar, [x0, top, x0 + stripe, top + h]));
                // what is left of its time: a thin line along the bottom
                let left = (n.left / n.total).clamp(0.0, 1.0);
                let line = self.text.solid(r, scene, [accent[0], accent[1], accent[2], (150.0 * a) as u8]);
                scene.overlays.push((line, [x0 + stripe, top + h - 2.0 * s, x0 + stripe + (w - stripe) * left, top + h]));
                let tx = x0 + stripe + pad;
                let t = self.text.label(r, scene, title, title_px, [accent[0], accent[1], accent[2], (255.0 * a) as u8]);
                scene.overlays.push((t.tex, [tx, top + pad * 0.6, tx + t.w as f32, top + pad * 0.6 + t.h as f32]));
                let mut ry = top + pad * 0.6 + th;
                for row in &rows {
                    let l = self.text.label(r, scene, row, px, [255, 255, 255, (240.0 * a) as u8]);
                    scene.overlays.push((l.tex, [tx, ry, tx + l.w as f32, ry + l.h as f32]));
                    ry += lh;
                }
                y = if up { top - gap } else { top + h + gap };
            }
        }
        if let Some(fps) = f.fps {
            let l = self.text.label(r, scene, &format!("{fps:.0} fps"), (13.0 * s) as u32, [255, 255, 255, 200]);
            let x = f.width - l.w as f32 - 12.0 * s;
            // (on a plate, as the notes are: over a bright sky the outline alone was not enough)
            let plate = self.text.plate(r, scene, 7);
            scene.overlays.push((plate, [x - 5.0 * s, 10.0 * s, x + l.w as f32 + 5.0 * s, 10.0 * s + l.h as f32]));
            scene.overlays.push((l.tex, [x, 10.0 * s, x + l.w as f32, 10.0 * s + l.h as f32]));
        }
        // --- the information bar, along the top in the middle: in as many rows as the room
        // it has needs
        self.info_rect = None;
        // (not under the pause menu's rail, from under which it stood out)
        let rail = f.menu.is_some() && f.menu_kind == MenuKind::Game && f.menu_head.is_none() && !f.vr;
        if let Some(info) = f.info.as_ref().filter(|_| !rail) {
            let px = (15.0 * s) as u32;
            let pad = 10.0 * s;
            let [x0, x1, y] = f.info_room.unwrap_or([12.0 * s, f.width - 12.0 * s, 8.0 * s]);
            let rows = info_rows(info, (x1 - x0 - pad * 2.0).max(1.0), |t| self.text.width(t, px as f32));
            let labels: Vec<Label> = rows.iter().map(|t| self.text.label(r, scene, t, px, [255, 255, 255, 0])).collect();
            let lw = labels.iter().map(|l| l.w).max().unwrap_or(0) as f32;
            let lh = labels.iter().map(|l| l.h).max().unwrap_or(0) as f32;
            let (w, h) = (lw + pad * 2.0, lh * labels.len() as f32 + pad * 0.8);
            let x = (x0 + x1 - w) * 0.5;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            for (k, l) in labels.iter().enumerate() {
                let (lx, ly) = (x + (w - l.w as f32) * 0.5, y + pad * 0.4 + k as f32 * lh);
                scene.overlays.push((l.tex, [lx, ly, lx + l.w as f32, ly + l.h as f32]));
            }
            self.info_rect = Some([x, y, x + w, y + h]);
        }
        let tutorial_w = (420.0 * s).min(f.width * 0.42);
        // --- the timetable window, on the right
        if let Some((title, rows)) = f.timetable.as_ref() {
            let px = (14.0 * s) as u32;
            let lh = px as f32 * 1.55;
            let w = (340.0 * s).min(f.width * 0.4);
            let shown = rows.len().min(((f.height * 0.7) / lh) as usize).max(1);
            // (from the stop before the next one on)
            let next = rows.iter().position(|r| r.2 == 1).unwrap_or(0);
            let first = next.saturating_sub(1).min(rows.len().saturating_sub(shown));
            let h = lh * (shown as f32 + 1.6);
            // (left of a tutorial page, which has the same corner)
            let beside = if f.tutorial.is_some() { tutorial_w + 12.0 * s } else { 0.0 };
            let x = (f.width - w - 16.0 * s - beside).max(16.0 * s);
            let y = corner_top;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, y, x + w, y + h]));
            let t = self.text.label(r, scene, &clip_to(&self.text, title, px as f32 * 1.1, w - 20.0 * s), (px as f32 * 1.1) as u32, [255, 255, 255, 0]);
            scene.overlays.push((t.tex, [x + 10.0 * s, y + 6.0 * s, x + 10.0 * s + t.w as f32, y + 6.0 * s + t.h as f32]));
            // (the names start after the widest time: "12:03-05" of a stop with a wait)
            let time_w = rows.iter().map(|r| self.text.width(&r.1, px as f32)).fold(0.0f32, f32::max).max(40.0 * s);
            let name_x = x + 10.0 * s + time_w + 12.0 * s;
            for (k, (name, time, state)) in rows.iter().skip(first).take(shown).enumerate() {
                let ry = y + lh * (k as f32 + 1.3);
                if *state == 1 {
                    let hl = self.text.plate(r, scene, 5);
                    scene.overlays.push((hl, [x + 4.0 * s, ry - 2.0 * s, x + w - 4.0 * s, ry + lh - 4.0 * s]));
                }
                let color = match state {
                    0 => [140, 140, 140, 0],
                    1 => [255, 200, 110, 0],
                    _ => [235, 235, 235, 0],
                };
                let tl = self.text.label(r, scene, time, px, color);
                scene.overlays.push((tl.tex, [x + 10.0 * s, ry, x + 10.0 * s + tl.w as f32, ry + tl.h as f32]));
                let nl = self.text.label(r, scene, &clip_to(&self.text, name, px as f32, x + w - name_x - 10.0 * s), px, color);
                scene.overlays.push((nl.tex, [name_x, ry, name_x + nl.w as f32, ry + nl.h as f32]));
            }
        }
        // --- a tutorial page, on the right
        if let Some((title, text, image, at, count)) = f.tutorial {
            let w = tutorial_w;
            let x = f.width - w - 16.0 * s;
            let mut y = corner_top;
            let top = y;
            let pad = 14.0 * s;
            let mut items: Vec<(TextureId, [f32; 4])> = Vec::new();
            if let Some(p) = image {
                let entry = self.images.entry(p.to_path_buf()).or_insert_with(|| {
                    omsi_texture::decode_file(p).ok().map(|img| {
                        let (iw, ih) = (img.width, img.height);
                        (r.add_texture(scene, &img, false), iw, ih)
                    })
                });
                if let Some((tex, iw, ih)) = *entry {
                    let dw = w - pad * 2.0;
                    let dh = dw * ih as f32 / iw.max(1) as f32;
                    items.push((tex, [x + pad, y + pad, x + pad + dw, y + pad + dh]));
                    y += dh + pad;
                }
            }
            y += pad * 0.6;
            let tp = (18.0 * s) as u32;
            if !title.is_empty() {
                for line in wrap(&self.text, title, tp as f32, w - pad * 2.0) {
                    let l = self.text.label(r, scene, &line, tp, [255, 200, 110, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32;
                }
                y += 6.0 * s;
            }
            let bp = (14.0 * s) as u32;
            let max_y = f.height - 60.0 * s;
            'text: for para in text.lines() {
                for line in wrap(&self.text, para, bp as f32, w - pad * 2.0) {
                    if y > max_y {
                        break 'text;
                    }
                    let l = self.text.label(r, scene, &line, bp, [235, 235, 235, 0]);
                    items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
                    y += l.h as f32 * 0.95;
                }
                y += 5.0 * s;
            }
            let tr = |t: &str| omsi_ui::tr(t).into_owned();
            let foot = format!("{} {}/{}   |   {}   |   {}   |   {}", tr("Page"), at + 1, count, tr("Enter next"), tr("Page Up back"), tr("Ctrl+T hide"));
            let l = self.text.label(r, scene, &foot, (12.0 * s) as u32, [150, 150, 150, 0]);
            y += 4.0 * s;
            items.push((l.tex, [x + pad, y, x + pad + l.w as f32, y + l.h as f32]));
            y += l.h as f32 + pad;
            let plate = self.text.plate(r, scene, 3);
            scene.overlays.push((plate, [x, top, x + w, y]));
            scene.overlays.extend(items);
        }
        // --- the game menu and its lists, in the middle over a dimmed picture
        self.anim_dt = dt.clamp(0.0, 0.1);
        self.text.flat = true;
        self.draw_menu(r, scene, f);
        self.text.flat = false;
        // --- the mouse-over name, right of the cursor
        self.vr_tooltip_overlay = None;
        if let Some(t) = f.tooltip.as_ref().filter(|t| !t.is_empty()) {
            let l = self.text.label(r, scene, t, (14.0 * s) as u32, [255, 255, 255, 235]);
            let mut x = f.cursor.0.clamp(0.0, f.width) + 16.0 * s;
            let mut y = f.cursor.1 + 2.0 * s;
            if x + l.w as f32 > f.width {
                x = (f.cursor.0.clamp(0.0, f.width) - 8.0 * s - l.w as f32).max(5.0 * s);
            }
            if y + l.h as f32 > f.height {
                y = f.height - l.h as f32;
            }
            if f.vr {
                self.vr_tooltip_overlay = Some(scene.overlays.len());
            } else {
                // (in a headset the text alone is placed in front of the eye)
                let plate = self.text.plate(r, scene, 7);
                scene.overlays.push((plate, [x - 5.0 * s, y, x + l.w as f32 + 5.0 * s, y + l.h as f32]));
            }
            scene.overlays.push((l.tex, [x, y, x + l.w as f32, y + l.h as f32]));
        }
        if let (Some((x, y)), false) = (f.steer_cross, f.vr) {
            // (the size of the system's crosshair cursor: 17 logical pixels)
            let tex = self.text.steer_cross(r, scene);
            let half = 8.5 * f.scale;
            scene.overlays.push((tex, [x - half, y - half, x + half, y + half]));
        }
        if f.vr {
            let pointer = self.text.vr_pointer(r, scene);
            self.vr_cursor_overlay = Some(scene.overlays.len());
            scene.overlays.push((pointer, [0.0, 0.0, 7.0 * s, 7.0 * s]));
        } else {
            self.vr_cursor_overlay = None;
        }
        self.text.end_frame(r, scene);
    }
}

/// The pixels (RGBA, `n`×`n`) of the steering cross: arms two pixels wide across the whole
/// picture, white, with a one-pixel dark edge so it shows on a bright sky and a dark road
/// alike, and a small gap in the middle that leaves the point itself visible.
fn steer_cross_pixels(n: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; n * n * 4];
    let c = n as f32 * 0.5;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = ((x as f32 + 0.5 - c).abs(), (y as f32 + 0.5 - c).abs());
            let (along, across) = if dx <= dy { (dy, dx) } else { (dx, dy) };
            let gap = along < 3.0;
            let px = if gap || along > c - 1.0 {
                [0, 0, 0, 0]
            } else if across <= 1.0 {
                [255, 255, 255, 255]
            } else if across <= 2.0 {
                [0, 0, 0, 200]
            } else {
                [0, 0, 0, 0]
            };
            rgba[(y * n + x) * 4..(y * n + x + 1) * 4].copy_from_slice(&px);
        }
    }
    rgba
}

impl Ui {
    /// The loading screen over a plain dark picture: the map's name in the middle, a thin
    /// bar of how far the start area got under it, and one quiet line (what is being done).
    pub fn loading(&mut self, r: &Renderer, scene: &mut Scene, width: f32, height: f32, scale: f32, title: &str, caption: &str, progress: f32) {
        let s = scale.max(0.5);
        let t = self.text.label(r, scene, title, (30.0 * s) as u32, [255, 255, 255, 0]);
        let cy = height * 0.5;
        let tx = (width - t.w as f32) * 0.5;
        let ty = cy - t.h as f32 - 14.0 * s;
        scene.overlays.push((t.tex, [tx, ty, tx + t.w as f32, ty + t.h as f32]));
        let bw = (320.0 * s).min(width * 0.6);
        let bh = (3.0 * s).max(2.0);
        let bx = (width - bw) * 0.5;
        let by = cy + 4.0 * s;
        let track = self.text.plate(r, scene, 1);
        scene.overlays.push((track, [bx, by, bx + bw, by + bh]));
        let fill = self.text.plate(r, scene, 2);
        let p = progress.clamp(0.0, 1.0);
        if p > 0.0 {
            scene.overlays.push((fill, [bx, by, bx + bw * p, by + bh]));
        }
        if !caption.is_empty() {
            let c = self.text.label(r, scene, caption, (13.0 * s) as u32, [200, 204, 210, 0]);
            let cx = (width - c.w as f32) * 0.5;
            let cy2 = by + bh + 14.0 * s;
            scene.overlays.push((c.tex, [cx, cy2, cx + c.w as f32, cy2 + c.h as f32]));
        }
        self.text.end_frame(r, scene);
    }
}


/// `text` broken into lines of at most `width` pixels, between words.
fn wrap(tc: &TextCache, text: &str, px: f32, width: f32) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let try_line = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
        if tc.width(&try_line, px) > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
            line = word.to_string();
        } else {
            line = try_line;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// `text` cut at the end to fit `width` pixels ("…").
fn clip_to(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    if tc.width(text, px) <= width {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let head = |n: usize| {
        let mut t: String = chars[..n].iter().collect();
        t.push('…');
        t
    };
    let (mut lo, mut hi) = (0usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        if tc.width_raw(&head(mid), px) <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    head(lo)
}

/// `text` in rows that fit `width` (as `measure` measures): broken between words, and inside a
/// word too long for a row. The rows after the first fit `width - indent` (they are drawn
/// indented, under the first).
fn wrap_rows(text: &str, width: f32, indent: f32, measure: &impl Fn(&str) -> f32) -> Vec<String> {
    let mut rows: Vec<String> = Vec::new();
    let mut cur = String::new();
    let room = |rows: &Vec<String>| if rows.is_empty() { width } else { width - indent };
    for word in text.split(' ') {
        let joined = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if measure(&joined) <= room(&rows) {
            cur = joined;
            continue;
        }
        if !cur.is_empty() {
            rows.push(std::mem::take(&mut cur));
        }
        // the word alone, cut where a row is full (a long link, a run of letters)
        let mut w: Vec<char> = word.chars().collect();
        while w.len() > 1 && measure(&w.iter().collect::<String>()) > room(&rows) {
            let mut k = w.len() - 1;
            while k > 1 && measure(&w[..k].iter().collect::<String>()) > room(&rows) {
                k -= 1;
            }
            rows.push(w[..k].iter().collect());
            w.drain(..k);
        }
        cur = w.into_iter().collect();
    }
    if !cur.is_empty() || rows.is_empty() {
        rows.push(cur);
    }
    rows
}

/// `text` cut at the start to fit (the end of what is being typed stays visible).
fn clip_left(tc: &TextCache, text: &str, px: f32, width: f32) -> String {
    let mut t: Vec<char> = text.chars().collect();
    while t.len() > 1 && tc.width(&t.iter().collect::<String>(), px) > width {
        t.remove(0);
    }
    t.into_iter().collect()
}

/// A chat line with its bad words starred out (rustrict: profanity, slurs and the usual
/// ways of writing around a filter, without a word list to keep).
pub fn filter_chat(text: &str) -> String {
    use rustrict::CensorStr;
    text.censor()
}

/// Fit all VR page buttons above the separately reserved Back button.
fn vr_settings_sidebar_step(available: f32, pages: usize, scale: f32) -> f32 {
    (available.max(0.0) / pages.max(1) as f32).min(42.0 * scale)
}

#[cfg(test)]
mod tests {
    /// The steering cross: white arms with a dark edge, open in the middle, nothing in the
    /// corners - the same in all four directions.
    #[test]
    fn the_steering_cross_is_a_white_cross_with_a_dark_edge() {
        let n = 34;
        let px = super::steer_cross_pixels(n);
        let at = |x: usize, y: usize| &px[(y * n + x) * 4..(y * n + x) * 4 + 4];
        let c = n / 2;
        assert_eq!(at(c, c)[3], 0, "the middle is open");
        assert_eq!(at(0, 0)[3], 0, "the corners are empty");
        for (x, y) in [(c, 6), (c, n - 7), (6, c), (n - 7, c)] {
            assert_eq!(at(x, y), [255, 255, 255, 255], "arm at {x},{y}");
        }
        assert_eq!(at(c + 2, 6)[..3], [0, 0, 0], "the arm's edge is dark");
    }

    use super::*;

    /// The information bar is broken between its parts into rows that fit the room it has,
    /// as many parts to a row as go in; a part wider than the room alone is a row of its own
    /// (#1164).
    #[test]
    fn the_information_bar_breaks_into_rows_that_fit() {
        let line = ["09:00:07", "0 km/h", "EXT 15 °C / INT 15 °C", "tank 80 %", "0 Passengers", "76 › Krankenhaus", "next: Bauernhof", "−6:52"].join(INFO_SEP);
        let w = |t: &str| t.chars().count() as f32 * 10.0;
        assert_eq!(info_rows(&line, 5000.0, w), vec![line.clone()]);
        let rows = info_rows(&line, 500.0, w);
        assert_eq!(rows, vec![
            format!("09:00:07{INFO_SEP}0 km/h{INFO_SEP}EXT 15 °C / INT 15 °C"),
            format!("tank 80 %{INFO_SEP}0 Passengers"),
            format!("76 › Krankenhaus{INFO_SEP}next: Bauernhof{INFO_SEP}−6:52"),
        ]);
        assert!(rows.iter().all(|r| w(r) <= 500.0));
        assert_eq!(rows.join(INFO_SEP), line);
        assert_eq!(info_rows(&line, 50.0, w).len(), 8);
    }

    #[test]
    fn a_server_notice_is_read_and_fades() {
        let (id, n) = Notice::parse("a1b2 5 warn Service 15/5: departure at 06:07 | depot").unwrap();
        assert_eq!(id, "a1b2");
        assert_eq!((n.kind, n.text.as_str(), n.total, n.left), (NoticeKind::Warn, "Service 15/5: departure at 06:07 | depot", 5.0, 5.0));
        // seconds held to 2 .. 30, an unknown kind is information
        assert_eq!(Notice::parse("x 0.5 info hi").unwrap().1.total, 2.0);
        assert_eq!(Notice::parse("x 600 whatever hi").unwrap().1, Notice { kind: NoticeKind::Info, text: "hi".into(), left: 30.0, total: 30.0 });
        // no text, no seconds, nothing: not a notice
        for bad in ["", "x", "x 5", "x 5 info", "x 5 info   ", "x five info hi", "x NaN info hi"] {
            assert!(Notice::parse(bad).is_none(), "{bad}");
        }
        // fades in, stays, fades out
        let mut n = Notice::parse("x 5 alert hi").unwrap().1;
        assert_eq!(n.alpha(), 0.0);
        n.left = 4.0;
        assert_eq!(n.alpha(), 1.0);
        n.left = 0.25;
        assert!((n.alpha() - 0.5).abs() < 1e-6);
        // three at most: the oldest goes
        let mut list = Vec::new();
        for k in 0..5 {
            push_notice(&mut list, Notice::parse(&format!("{k} 5 info n{k}")).unwrap().1);
        }
        assert_eq!(list.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(), ["n2", "n3", "n4"]);
    }

    #[test]
    fn centre_hud_moves_menu_controls_and_chat_hitboxes_together() {
        let mut ui = Ui::new().unwrap();
        ui.menu_rects.push([20.0, 100.0, 900.0, 140.0]);
        ui.menu_ctl.push(Some([600.0, 110.0, 860.0, 130.0]));
        ui.menu_side.push([30.0, 100.0, 200.0, 140.0]);
        ui.dd_rects.push([650.0, 150.0, 850.0, 180.0]);
        ui.menu_arrows.push(Some([600.0, 640.0, 820.0]));
        ui.chat.rect = [10.0, 800.0, 400.0, 1000.0];
        ui.dd_scroll = Some(([850.0, 150.0, 860.0, 300.0], [844.0, 160.0, 864.0, 200.0]));
        ui.menu_pane_scroll = Some((
            [880.0, 150.0, 890.0, 700.0],
            [874.0, 160.0, 894.0, 220.0],
            40,
            10,
        ));
        ui.shift_hitboxes(1920.0);
        let track = ui.menu_ctl[0].unwrap();
        assert_eq!((2650.0 - track[0]) / (track[2] - track[0]), 0.5);
        assert!(ui.chat.contains(2020.0, 900.0));
        assert!(!ui.chat.contains(100.0, 900.0));
        assert_eq!(ui.dd_rects[0], [2570.0, 150.0, 2770.0, 180.0]);
        assert_eq!(ui.menu_arrows[0], Some([2520.0, 2560.0, 2740.0]));
        assert_eq!(ui.dd_scroll.unwrap().0[0], 2770.0);
        assert_eq!(ui.menu_pane_scroll.unwrap().1[0], 2794.0);
        ui.shift_hitboxes(-1920.0);
        assert_eq!(ui.menu_ctl[0], Some([600.0, 110.0, 860.0, 130.0]));
    }

    #[test]
    fn a_long_chat_line_goes_on_in_rows() {
        // (10 px a character)
        let m = |t: &str| t.chars().count() as f32 * 10.0;
        // fits: one row, as it was
        assert_eq!(wrap_rows("Anton: hello", 200.0, 20.0, &m), ["Anton: hello"]);
        assert_eq!(wrap_rows("", 200.0, 20.0, &m), [""]);
        // between words; the rows after the first are 20 px narrower
        let rows = wrap_rows("Admin (private): take tour 13/1 at 04:47 from the depot", 200.0, 20.0, &m);
        assert_eq!(rows, ["Admin (private):", "take tour 13/1 at", "04:47 from the", "depot"]);
        assert!(rows[0].chars().count() <= 20 && rows[1..].iter().all(|r| r.chars().count() <= 18));
        // nothing lost: the words come back in order
        assert_eq!(rows.join(" "), "Admin (private): take tour 13/1 at 04:47 from the depot");
        // a word longer than a row is cut inside
        let rows = wrap_rows("x: https://example.org/a/very/long/link", 100.0, 20.0, &m);
        assert_eq!(rows, ["x:", "https://", "example.", "org/a/ve", "ry/long/", "link"]);
        assert_eq!(rows.concat(), "x:https://example.org/a/very/long/link");
        // a box narrower than a character still ends (a character a row)
        assert_eq!(wrap_rows("abc", 5.0, 0.0, &m), ["a", "b", "c"]);
    }

    #[test]
    fn vr_settings_sidebar_keeps_back_clear() {
        for scale in [0.5, 1.0, 2.0] {
            for height in [220.0, 440.0] {
                let pages_top = (72.0 + 6.0) * scale;
                let back_top = (height - PAD - 6.0 - 38.0) * scale;
                for pages in [7, 8, 10] {
                    let step = vr_settings_sidebar_step(back_top - 6.0 * scale - pages_top, pages, scale);
                    let last_bottom = pages_top + (pages - 1) as f32 * step + (step - 4.0 * scale).max(1.0);
                    assert!(last_bottom < back_top);
                }
            }
        }
        assert_eq!(vr_settings_sidebar_step(1000.0, 8, 1.0), 42.0);
    }

    #[test]
    fn text_renders_with_an_outline() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        let img = render_text(&f, "Savva: hi", 20.0, [255, 255, 255, 220]);
        assert!(img.width > 40 && img.height > 14);
        // white text and dark outline pixels are both there
        let px: Vec<&[u8]> = img.rgba.chunks(4).collect();
        assert!(px.iter().any(|p| p[3] > 200 && p[0] > 200));
        assert!(px.iter().any(|p| p[3] > 100 && p[0] < 40));
    }

    /// The Esc menu in Chinese, Korean and Thai: the system's fonts, not boxes.
    #[test]
    fn scripts_roboto_lacks_come_from_the_system() {
        let f = FontVec::try_from_vec(ROBOTO.to_vec()).unwrap();
        for t in ["继续", "繼續", "계속", "ดำเนินการต่อ"] {
            if t.chars().next().and_then(omsi_ui::text::fallback_font).is_none() {
                continue;
            }
            for c in t.chars() {
                let g = font_for(&f, c);
                assert!(!std::ptr::eq(g, &f) && g.glyph_id(c).0 != 0, "{t}: {c}");
            }
            let img = render_text(&f, t, 20.0, [255, 255, 255, 220]);
            let ink = img.rgba.chunks(4).filter(|p| p[3] > 128 && p[0] > 128).count();
            assert!(ink > 30, "{t}: {ink}");
        }
    }

    /// Laid out for 1080p: a lower window as it always was, a taller one in proportion.
    #[test]
    fn the_interface_grows_with_tall_windows() {
        assert_eq!(size_factor(900.0, 1.0, 1.0, true), 1.0);
        assert_eq!(size_factor(1080.0, 1.0, 1.0, true), 1.0);
        assert_eq!(size_factor(2160.0, 1.0, 1.0, true), 2.0);
        assert_eq!(size_factor(4320.0, 1.0, 1.0, true), 2.0);
        // (4K at 150 % display scaling is 1440 logical lines: with the screen's scale the
        // same size as at 100 %)
        assert!((1.5 * size_factor(2160.0, 1.5, 1.0, true) - 2.0).abs() < 1e-5);
        assert_eq!(size_factor(1080.0, 1.0, 1.5, true), 1.5);
        assert_eq!(size_factor(2160.0, 1.0, 0.5, true), 1.0);
        // (switched off: the size alone)
        assert_eq!(size_factor(2160.0, 1.0, 1.0, false), 1.0);
        assert_eq!(size_factor(2160.0, 1.0, 1.25, false), 1.25);
    }

    /// A thinned panel's light texts get an outline; dark ones - the highlighted menu line's,
    /// on the solid accent - do not (it came out as a smear round them, like a shadow).
    #[test]
    fn panel_texts_get_an_outline_only_where_it_helps() {
        // (full panels: as asked)
        assert_eq!(outline_for([235, 235, 235, 0], 1.0), 0);
        // (thinned: the light text gets one, the dark text none)
        assert!(outline_for([235, 235, 235, 0], 0.47) > 100);
        assert!(outline_for([140, 140, 140, 0], 0.47) > 100);
        assert_eq!(outline_for([20, 20, 20, 0], 0.47), 0);
        // (texts with an outline of their own keep it)
        assert_eq!(outline_for([255, 255, 255, 235], 0.47), 235);
    }

    /// The opacity's default is the design; below it the backgrounds fade, never quite away.
    #[test]
    fn backgrounds_follow_the_opacity_setting() {
        assert_eq!(backdrop(0.85), 1.0);
        assert!((backdrop(0.425) - 0.5).abs() < 1e-6);
        assert_eq!(backdrop(0.2), 0.3);
        assert!(backdrop(1.0) > 1.0);
    }

    #[test]
    fn the_game_menu_grows_with_the_interface_size_as_far_as_it_fits() {
        // (it ignored the setting: 1.5 drew the menu as 1.0 while the HUD grew)
        assert_eq!(super::menu_scale_for(1.0, 1.0, 1280.0, 720.0, false), 1.0);
        let s = super::menu_scale_for(1.0, 1.5, 1280.0, 720.0, false);
        assert!(s > 1.1 && s < 1.5 && 880.0 * s + 24.0 <= 1280.0 && 600.0 * s <= 720.0 * 0.94 + 0.01, "{s}");
        assert_eq!(super::menu_scale_for(1.0, 1.5, 3840.0, 2160.0, false), 1.5);
        // never smaller than it always was, and the headset keeps its own size
        assert_eq!(super::menu_scale_for(1.0, 1.5, 800.0, 500.0, false), 1.0);
        assert_eq!(super::menu_scale_for(1.0, 1.5, 1920.0, 1080.0, true), 1.0);
    }

    #[test]
    fn destinations_split_into_code_and_name() {
        assert_eq!(super::code_and_name(" 13  BETRIEBSFAHRT"), Some(("13", "BETRIEBSFAHRT")));
        assert_eq!(super::code_and_name("1001  Blanko.tga"), Some(("1001", "Blanko.tga")));
        assert_eq!(super::code_and_name("Route number: 5..."), None);
        assert_eq!(super::code_and_name("Line 76  (1 tours)"), None);
    }

    #[test]
    fn chat_filter_stars_out_swearing() {
        assert_ne!(filter_chat("you are a fucking idiot"), "you are a fucking idiot");
        assert_eq!(filter_chat("next stop Rathaus Spandau"), "next stop Rathaus Spandau");
    }
}
