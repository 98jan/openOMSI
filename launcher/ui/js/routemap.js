// The route preview, drawn as the game's own city map draws it (read by the game program,
// `route_map`): the map's ground, water, woods and houses (its layers, `d.layers`), every paved
// surface and road, the trip's route in blue on the very lanes the game drives, its stops with
// their times, and the entry points - one clicked is where the bus starts. Dragged to move,
// the wheel to zoom where the mouse is.

import { html, useEffect, useRef, useState } from '../vendor/preact-htm.js';
import { call } from './api.js';
import { t, plain } from './i18n.js';
import { Spinner } from './ui.js';

const cache = new Map();

/** The stop objects stand beside the road; their dots go onto the route itself: the nearest
 * point of the route's lanes (within 40 m - a stop the route misses stays where it is). */
function onRoute(d) {
  for (const s of d.stops) {
    let best = null;
    let bd = 40 * 40;
    for (const p of d.route) {
      for (let i = 0; i + 3 < p.length; i += 2) {
        const [ax, ay, bx, by] = [p[i], p[i + 1], p[i + 2], p[i + 3]];
        const vx = bx - ax;
        const vy = by - ay;
        const l = vx * vx + vy * vy || 1;
        const k = Math.max(0, Math.min(1, ((s.at[0] - ax) * vx + (s.at[1] - ay) * vy) / l));
        const x = ax + vx * k;
        const y = ay + vy * k;
        const dd = (x - s.at[0]) ** 2 + (y - s.at[1]) ** 2;
        if (dd < bd) (bd = dd), (best = [x, y]);
      }
    }
    if (best) s.at = best;
  }
  return d;
}



// The picture under the route is drawn into a canvas of its own, a little larger than the view,
// a slice of a few milliseconds at a time between frames: moving and zooming only scale and
// shift the last finished one, so a frame never waits for the whole city to be drawn again.
// Everything is cut into squares of the map, and only the squares in view are drawn.
const CHUNK = 1024;
const CELL = 400; // metres
const MARGIN = 1.5; // the picture's size, by the view's
const SLICE = 8; // ms of drawing between two frames

/** Triangles (x, y, x, y, x, y) in runs of CHUNK, each with its bounds. */
function tris(t) {
  const out = [];
  for (let s = 0; s + 5 < t.length; s += CHUNK * 6) {
    const e = Math.min(s + CHUNK * 6, t.length - 5);
    const lo = [Infinity, Infinity];
    const hi = [-Infinity, -Infinity];
    for (let i = s; i < e; i += 2) {
      const x = t[i];
      const y = t[i + 1];
      if (x < lo[0]) lo[0] = x;
      if (x > hi[0]) hi[0] = x;
      if (y < lo[1]) lo[1] = y;
      if (y > hi[1]) hi[1] = y;
    }
    out.push({ t, s, e, lo, hi, path: null });
  }
  return out;
}

function pathOf(c) {
  if (c.path) return c.path;
  const { t, s, e } = c;
  const path = new Path2D();
  for (let i = s; i < e; i += 6) {
    path.moveTo(t[i], t[i + 1]);
    path.lineTo(t[i + 2], t[i + 3]);
    path.lineTo(t[i + 4], t[i + 5]);
    path.closePath();
  }
  return (c.path = path);
}

/** A line (or a ring) into the Path2D of the map square it starts in (`cells`: square -> path). */
function into(cells, p, ring = false, dx = 0, dy = 0) {
  if (p.length < 2) return;
  const k = `${Math.floor(p[0] / CELL)},${Math.floor(p[1] / CELL)}`;
  let c = cells.get(k);
  if (!c) cells.set(k, (c = { path: new Path2D(), lo: [Infinity, Infinity], hi: [-Infinity, -Infinity] }));
  for (let i = 0; i + 1 < p.length; i += 2) {
    const x = p[i] + dx;
    const y = p[i + 1] + dy;
    if (i) c.path.lineTo(x, y);
    else c.path.moveTo(x, y);
    if (x < c.lo[0]) c.lo[0] = x;
    if (x > c.hi[0]) c.hi[0] = x;
    if (y < c.lo[1]) c.lo[1] = y;
    if (y > c.hi[1]) c.hi[1] = y;
  }
  if (ring) c.path.closePath();
}

// The map package's surface (`d.package`, see docs/MAP_PACKAGE.md): its triangles and lines in
// map metres, made once and drawn under a transform that follows the view.
const FILL_KEYS = ['water', 'green', 'built', 'grass', 'forest', 'sport', 'farm']; // by class
const FILL_LOOK = ['water', 'park', 'built', 'grass', 'forest', 'sport', 'field'];
const FILL_ORDER = [2, 6, 3, 1, 4, 5, 0]; // under everything: built-up land first, water last

function surfacePaths(pkg) {
  if (pkg.paths) return pkg.paths;
  const fills = pkg.fills.map(tris);
  // a river's width tells river from stream; the railway and the borders are one path each
  const lines = { river: new Path2D(), stream: new Path2D(), rail: new Path2D(), border: new Path2D() };
  for (const l of pkg.lines) {
    const path = l.c === 0 ? (l.w >= 5 ? lines.river : lines.stream) : l.c === 1 ? lines.rail : l.c === 2 ? lines.border : null;
    if (!path) continue;
    for (let i = 0; i + 1 < l.p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, l.p[i], l.p[i + 1]);
  }
  return (pkg.paths = { fills, lines });
}

/** Map metres (y up) onto the canvas: widths are then pixels times `mpp`. */
function toMap(g, v, W, H) {
  g.translate(W / 2 - v.cx / v.mpp, H / 2 + v.cy / v.mpp);
  g.scale(1 / v.mpp, -1 / v.mpp);
  g.lineCap = g.lineJoin = 'round';
}

/** The surface under the roads: ground, areas, then rivers, rail and borders. */
function* drawSurface(g, pkg, v, W, H, seen) {
  const c = LOOK === LOOKS.light && pkg.colors;
  const { fills, lines } = surfacePaths(pkg);
  g.save();
  toMap(g, v, W, H);
  for (const k of FILL_ORDER) {
    g.fillStyle = g.strokeStyle = c ? c[FILL_KEYS[k]] : LOOK[FILL_LOOK[k]];
    g.lineWidth = 0.8 * v.mpp; // (no seams between neighbouring triangles)
    for (const ch of seen(fills[k])) {
      g.fill(pathOf(ch));
      g.stroke(pathOf(ch));
      yield;
    }
  }
  const stroke = (path, color, px) => {
    g.strokeStyle = color;
    g.lineWidth = px * v.mpp;
    g.stroke(path);
  };
  stroke(lines.river, c ? c.river : LOOK.river, 6);
  stroke(lines.stream, c ? c.river : LOOK.river, 2.4);
  yield;
  stroke(lines.rail, c ? c.rail_edge : LOOK.rail, 3.4);
  stroke(lines.rail, c ? c.rail : LOOK.railDash, 1.6);
  g.globalAlpha = 0.7;
  stroke(lines.border, c ? c.border : LOOK.border, 1.2);
  g.restore();
}

const LOOKS = {
  light: {
    land: '#d8ecc8', grass: '#d8ecc8', forest: '#c3e0ae', field: '#eef0d8', paved: '#eceae4', sand: '#f3ead2', water: '#a6d3f5',
    park: '#d3eac1', built: '#efebe3', sport: '#cfe9c8', river: '#a6d3f5', border: '#b9a9c9',
    road: '#ffffff', roadEdge: '#dcd8cf', main: '#fbe08a', mainEdge: '#e5bd55', foot: '#f8f7f3', footEdge: '#e4e1d9', area: '#efedea', areaEdge: '#dedbd6',
    roof: '#e9e6df', roofEdge: '#d6d1c7', shadow: 'rgba(60,64,52,0.10)', rail: '#b2afa8', railDash: '#ffffff', tram: '#a59fb2',
    route: '#0a84ff', routeEdge: '#0060df', pin: '#ffffff', card: 'rgba(255,255,255,0.96)', cardShadow: 'rgba(0,0,0,0.16)', text: '#1c1c1e', dim: '#6e6e73', entry: '#ff9f0a',
  },
  dark: {
    land: '#22322a', grass: '#22322a', forest: '#1f3527', field: '#2b2f29', paved: '#2e3033', sand: '#36332b', water: '#1d3550',
    park: '#213628', built: '#2c2d31', sport: '#22362b', river: '#1d3550', border: '#6b5d7a',
    road: '#4a4d53', roadEdge: '#323438', main: '#8a7a4e', mainEdge: '#5c5236', foot: '#34363a', footEdge: '#2c2e31', area: '#313337', areaEdge: '#2a2c2f',
    roof: '#3a3c41', roofEdge: '#46494f', shadow: 'rgba(0,0,0,0.28)', rail: '#5d5f64', railDash: '#2f3134', tram: '#6a6478',
    route: '#0a84ff', routeEdge: '#0a5bd1', pin: '#f2f2f7', card: 'rgba(44,44,46,0.96)', cardShadow: 'rgba(0,0,0,0.5)', text: '#f2f2f7', dim: '#98989d', entry: '#ff9f0a',
  },
};
let LOOK = LOOKS.light;
// land classes in the order they are laid: grass, field, sand, paved, forest, water
const LAND = [[0, 'grass'], [2, 'field'], [4, 'sand'], [3, 'paved'], [1, 'forest'], [5, 'water']];

/** The layers' triangles, houses and tracks in map metres, made once. */
function layerPaths(l) {
  if (l.paths) return l.paths;
  const houses = new Map();
  const shadows = new Map();
  for (const h of l.houses) {
    into(houses, h.p, true);
    // (the shadow falls south-east, longer for a taller house)
    const k = Math.min(h.h, 40) * 0.12;
    into(shadows, h.p, true, k, -k);
  }
  const rail = new Path2D();
  const tram = new Path2D();
  for (const x of l.lines) {
    const path = x.c === 0 ? rail : x.c === 1 ? tram : null;
    if (!path) continue;
    for (let i = 0; i + 1 < x.p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, x.p[i], x.p[i + 1]);
  }
  return (l.paths = { land: l.land.map(tris), paved: l.paved.map(tris), houses: [...houses.values()], shadows: [...shadows.values()], rail, tram });
}

/** The ground, the paved surfaces, the tracks and the houses, under the roads' middles. */
function* drawLayers(g, l, v, W, H, seen) {
  const P = layerPaths(l);
  g.save();
  toMap(g, v, W, H);
  const fill = function* (chunks, color, seam = 0.8) {
    g.fillStyle = g.strokeStyle = color;
    g.lineWidth = seam * v.mpp; // (no seams between neighbouring triangles)
    for (const c of seen(chunks)) {
      g.fill(pathOf(c));
      g.stroke(pathOf(c));
      yield;
    }
  };
  for (const [k, key] of LAND) yield* fill(P.land[k], LOOK[key]);
  const near = v.mpp < 2.5;
  // pavements and yards (near), carriageways: an edge, then the surface (the edge only near: from
  // afar the roads are drawn over them with edges of their own, and edging every triangle of a
  // whole city took seconds)
  const paved = near ? [[1, 'foot', 'footEdge'], [2, 'area', 'areaEdge'], [0, 'road', 'roadEdge']] : [[2, 'area', 'areaEdge'], [0, 'road', 'roadEdge']];
  for (const [k, f, e] of paved) {
    if (near) {
      g.strokeStyle = LOOK[e];
      g.lineWidth = 2 * v.mpp;
      for (const c of seen(P.paved[k])) {
        g.stroke(pathOf(c));
        yield;
      }
    }
    yield* fill(P.paved[k], LOOK[f]);
  }
  g.strokeStyle = LOOK.rail;
  g.lineWidth = Math.max(3.4, 3.6 / v.mpp) * v.mpp;
  g.stroke(P.rail);
  if (near) {
    g.setLineDash([9, 9]);
    g.strokeStyle = LOOK.railDash;
    g.lineWidth = Math.max(1.4, 1.3 / v.mpp) * v.mpp;
    g.stroke(P.rail);
    g.setLineDash([]);
  }
  g.strokeStyle = LOOK.tram;
  g.lineWidth = 1.3 * v.mpp;
  g.stroke(P.tram);
  yield;
  if (v.mpp < 6) {
    g.fillStyle = LOOK.shadow;
    for (const c of seen(P.shadows)) g.fill(c.path);
    yield;
    g.fillStyle = LOOK.roof;
    g.strokeStyle = LOOK.roofEdge;
    g.lineWidth = 0.8 * v.mpp;
    for (const c of seen(P.houses)) {
      g.fill(c.path);
      g.stroke(c.path);
    }
    yield;
  }
  g.restore();
}

const surfaces = new Map();
let drawn = 0;

export async function readRouteMap(map, date, trip, surface = true) {
  return JSON.parse(new TextDecoder().decode(await call('route_map', { map, date, trip, surface })));
}

const surfaceNow = new Map();

function surfaceOf(map, date) {
  const at = `${map}|${date}`;
  if (!surfaces.has(at)) {
    surfaces.set(at, readRouteMap(map, date, '', true).then(
      (d) => {
        const s = { package: d.package, layers: d.layers };
        surfaceNow.set(at, s);
        return s;
      },
      () => (surfaces.delete(at), null),
    ));
  }
  return surfaces.get(at);
}

function load(map, date, trip) {
  const key = `${map}|${date}|${trip}`;
  if (!cache.has(key)) cache.set(key, readRouteMap(map, date, trip, false).then(onRoute, (e) => ({ error: e.message })));
  return cache.get(key);
}

const withSurface = (d, s) => ({ ...d, package: s?.package ?? d.package, layers: s?.layers ?? d.layers, id: ++drawn });

function roadPaths(d) {
  if (d.roadPaths) return d.roadPaths;
  const groups = new Map();
  for (const r of d.roads) {
    const w = Math.round(r.w * 2) / 2;
    const k = `${r.main ? 1 : 0}|${w}`;
    if (!groups.has(k)) groups.set(k, { main: !!r.main, w, cells: new Map() });
    into(groups.get(k).cells, r.p);
  }
  return (d.roadPaths = [...groups.values()].map((g) => ({ ...g, cells: [...g.cells.values()] })));
}

function* drawRoads(g, d, v, W, H, seen) {
  const paved = !!d.layers && v.mpp < 2.5;
  g.save();
  toMap(g, v, W, H);
  for (let pass = 0; pass < 4; pass++) {
    const main = pass % 2 === 1;
    const top = pass > 1;
    const gold = main && !paved;
    g.strokeStyle = top ? (gold ? LOOK.main : LOOK.road) : gold ? LOOK.mainEdge : LOOK.roadEdge;
    for (const r of roadPaths(d)) {
      if (r.main !== main) continue;
      const w = paved ? Math.max(r.w - 1.2, 1.5) : r.w;
      g.lineWidth = Math.max((w + (top ? 0 : paved ? 0 : 1.2)) / v.mpp, top ? (main ? 2.2 : 1.4) : main ? 3.4 : 2.6) * v.mpp;
      for (const c of seen(r.cells, g.lineWidth)) g.stroke(c.path);
      yield;
    }
  }
  g.restore();
}

/** Everything under the route, for view `v` on a W x H canvas (steps: see `sliced`). */
function* drawBase(g, d, v, W, H, dpr) {
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.fillStyle = LOOK.land;
  g.fillRect(0, 0, W, H);
  const x0 = v.cx - (W / 2 + 4) * v.mpp;
  const x1 = v.cx + (W / 2 + 4) * v.mpp;
  const y0 = v.cy - (H / 2 + 4) * v.mpp;
  const y1 = v.cy + (H / 2 + 4) * v.mpp;
  const seen = (chunks, pad = 0) => (chunks || []).filter((c) => c.hi[0] + pad >= x0 && c.lo[0] - pad <= x1 && c.hi[1] + pad >= y0 && c.lo[1] - pad <= y1);
  if (d.package) yield* drawSurface(g, d.package, v, W, H, seen);
  if (d.layers) yield* drawLayers(g, d.layers, v, W, H, seen);
  yield* drawRoads(g, d, v, W, H, seen);
}

/** Run a drawing a slice at a time, one slice a frame; `done` when it is finished. Returns
 * what stops it. */
function sliced(steps, look, done) {
  let stopped = false;
  const step = () => {
    if (stopped) return;
    const end = performance.now() + SLICE;
    const was = LOOK;
    LOOK = look;
    try {
      while (performance.now() < end) {
        if (steps.next().done) {
          stopped = true;
          done();
          return;
        }
      }
    } finally {
      LOOK = was;
    }
    // (a hidden window gets no frames: the timer goes on then)
    let next = 0;
    const go = () => {
      if (next++) return;
      setTimeout(step, 0);
    };
    requestAnimationFrame(go);
    setTimeout(go, 100);
  };
  setTimeout(step, 0);
  return () => (stopped = true);
}

/** The route in map metres, made once. */
function routePath(d) {
  if (d.routePath) return d.routePath;
  const path = new Path2D();
  for (const p of d.route) for (let i = 0; i + 1 < p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, p[i], p[i + 1]);
  return (d.routePath = path);
}

export function RouteMap({ map, date, trip, stops = [], entry = -1, onEntry, height = 340, inset }) {
  const box = useRef();
  const cv = useRef();
  const [data, setData] = useState(null);
  const view = useRef(null);
  // the finished pictures: of the view (`base`) and of the whole map (`whole`, for what lies
  // beyond the view's picture while it is moved or zoomed out), and the one being drawn
  const base = useRef(null);
  const whole = useRef(null);
  const job = useRef(null);
  const spare = useRef(null);
  const busy = useRef(false);
  const settle = useRef(0);
  const frame = useRef(0);
  const [hover, setHover] = useState(-1);

  useEffect(() => {
    if (!map) return;
    let alive = true;
    const surface = surfaceOf(map, date);
    load(map, date, trip).then((d) => {
      if (!alive) return;
      const now = surfaceNow.get(`${map}|${date}`);
      setData(d.error ? d : withSurface(d, now));
      view.current = null;
      if (d.error || now) return;
      surface.then((s) => alive && s && setData((cur) => (cur && !cur.error ? withSurface(cur, s) : cur)));
    });
    return () => (alive = false);
  }, [map, date, trip]);

  /** Start drawing the picture `v` needs, unless it is there or under way. */
  const refresh = (d, v, W, H, dpr) => {
    const key = `${d.id}|${LOOK === LOOKS.light}|${W}x${H}@${dpr}`;
    const same = (x) => x && x.key === key && x.view.cx === v.cx && x.view.cy === v.cy && x.view.mpp === v.mpp;
    if (same(base.current)) return wholeMap(d, W, H);
    const j = job.current;
    // (a picture of another zoom is no use any more; one of another place still is, for a while)
    if (j && (j.key !== key || j.view.mpp !== v.mpp)) {
      j.stop();
      job.current = null;
      if (j.key === 'whole') whole.current = { ...whole.current, making: null };
    }
    if (busy.current || job.current) return;
    const BW = Math.round(W * MARGIN);
    const BH = Math.round(H * MARGIN);
    const canvas = spare.current || document.createElement('canvas');
    spare.current = null;
    Object.assign(canvas, { width: Math.round(BW * dpr), height: Math.round(BH * dpr) });
    const at = { ...v };
    const look = LOOK;
    const made = { canvas, key, view: at, W: BW, H: BH, map };
    job.current = {
      key,
      view: at,
      stop: sliced(drawBase(canvas.getContext('2d'), d, at, BW, BH, dpr), look, () => {
        job.current = null;
        if (base.current && base.current.canvas !== whole.current?.canvas) spare.current = base.current.canvas;
        base.current = made;
        request();
      }),
    };
  };

  /** The whole map, once, at a scale that fits it in about 1600 pixels. */
  const wholeMap = (d, W, H) => {
    const key = `${d.id}|${LOOK === LOOKS.light}`;
    if (whole.current?.key === key || whole.current?.making === key || job.current) return;
    const span = Math.max(d.hi[0] - d.lo[0], d.hi[1] - d.lo[1], 1) * 1.2;
    const mpp = span / 1600;
    const OW = Math.ceil((d.hi[0] - d.lo[0]) * 1.2 / mpp) + 2;
    const OH = Math.ceil((d.hi[1] - d.lo[1]) * 1.2 / mpp) + 2;
    const canvas = document.createElement('canvas');
    Object.assign(canvas, { width: OW, height: OH });
    const at = { cx: (d.lo[0] + d.hi[0]) / 2, cy: (d.lo[1] + d.hi[1]) / 2, mpp };
    whole.current = { ...whole.current, making: key };
    job.current = {
      key: 'whole',
      view: { mpp: NaN },
      stop: sliced(drawBase(canvas.getContext('2d'), d, at, OW, OH, 1), LOOK, () => {
        job.current = null;
        whole.current = { canvas, key, view: at, W: OW, H: OH, map };
        request();
      }),
    };
  };

  // the picture: the finished pictures scaled to the view, the route, the stops and the entry
  // points over them
  const draw = () => {
    const c = cv.current;
    const d = data;
    if (!c || !d || d.error) return;
    const dpr = window.devicePixelRatio || 1;
    LOOK = document.documentElement.dataset.theme === 'light' ? LOOKS.light : LOOKS.dark;
    const W = box.current.clientWidth;
    const H = box.current.clientHeight;
    if (c.width !== Math.round(W * dpr) || c.height !== Math.round(H * dpr)) Object.assign(c, { width: Math.round(W * dpr), height: Math.round(H * dpr) });
    if (!view.current) {
      // the trip's own route when there is one, else the map
      let lo = d.lo;
      let hi = d.hi;
      if (d.route.length) {
        lo = [Infinity, Infinity];
        hi = [-Infinity, -Infinity];
        for (const p of d.route) for (let i = 0; i < p.length; i += 2) {
          lo = [Math.min(lo[0], p[i]), Math.min(lo[1], p[i + 1])];
          hi = [Math.max(hi[0], p[i]), Math.max(hi[1], p[i + 1])];
        }
      }
      const k = inset?.(box.current) || { l: 0, t: 0, r: 0, b: 0 };
      const fw = Math.max(W - k.l - k.r, W / 3);
      const fh = Math.max(H - k.t - k.b, H / 3);
      const span = Math.max((hi[0] - lo[0]) / Math.max(fw - (inset ? 220 : 80), fw / 2), (hi[1] - lo[1]) / Math.max(fh - (inset ? 90 : 60), fh / 2), 0.2);
      const ox = Math.min(k.l, W - fw) + fw / 2 - W / 2;
      const oy = Math.min(k.t, H - fh) + fh / 2 - H / 2;
      view.current = { cx: (lo[0] + hi[0]) / 2 - ox * span, cy: (lo[1] + hi[1]) / 2 + oy * span, mpp: span };
    }
    const v = view.current;
    refresh(d, v, W, H, dpr);
    const g = c.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.fillStyle = LOOK.land;
    g.fillRect(0, 0, W, H);
    for (const b of [whole.current, base.current]) {
      if (!b?.canvas || b.map !== map) continue;
      const k = b.view.mpp / v.mpp;
      g.drawImage(b.canvas, W / 2 + (b.view.cx - v.cx) / v.mpp - (b.W / 2) * k, H / 2 - (b.view.cy - v.cy) / v.mpp - (b.H / 2) * k, b.W * k, b.H * k);
    }
    const X = (x) => W / 2 + (x - v.cx) / v.mpp;
    const Y = (y) => H / 2 - (y - v.cy) / v.mpp;
    g.save();
    toMap(g, v, W, H);
    for (const [color, wide] of [[LOOK.routeEdge, 7], [LOOK.route, 4.6]]) {
      g.strokeStyle = color;
      g.lineWidth = wide * v.mpp;
      g.stroke(routePath(d));
    }
    g.restore();
    g.lineCap = g.lineJoin = 'round';
    // stops: white pins with the bus's blue, a card with the time and name beside
    g.font = '600 11.5px Inter, sans-serif';
    // (a label that would cover one already written goes to the left of its stop, else away)
    const taken = [];
    const free = (x, y, w) => !taken.some((r) => x < r[0] + r[2] && x + w > r[0] && y < r[1] + 20 && y + 20 > r[1]);
    for (const s of d.stops) {
      const x = X(s.at[0]);
      const y = Y(s.at[1]);
      if (x < -300 || x > W + 300 || y < -30 || y > H + 30) continue;
      g.fillStyle = LOOK.pin;
      g.beginPath();
      g.arc(x, y, 5.5, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = LOOK.route;
      g.beginPath();
      g.arc(x, y, 3.6, 0, Math.PI * 2);
      g.fill();
      const st = stops[s.k];
      if (st) {
        const label = `${st.time}  ${st.name}`;
        const w = g.measureText(label).width + 14;
        const lx = [x + 10, x - 10 - w].find((lx) => free(lx, y - 10, w));
        if (lx === undefined) continue;
        taken.push([lx, y - 10, w]);
        g.save();
        g.shadowColor = LOOK.cardShadow;
        g.shadowBlur = 6;
        g.fillStyle = LOOK.card;
        g.beginPath();
        g.roundRect(lx, y - 10, w, 20, 7);
        g.fill();
        g.restore();
        g.fillStyle = LOOK.route;
        g.fillText(plain(st.time), lx + 7, y + 4);
        g.fillStyle = LOOK.text;
        g.fillText(plain(st.name), lx + 7 + g.measureText(`${st.time}  `).width, y + 4);
      }
    }
    // entry points: amber, the hovered one ringed grey, the chosen one white
    d.entries.forEach((e, i) => {
      const x = X(e.at[0]);
      const y = Y(e.at[1]);
      if (x < -20 || x > W + 20 || y < -20 || y > H + 20) return;
      g.fillStyle = LOOK.pin;
      g.beginPath();
      g.arc(x, y, 5.5, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = LOOK.entry;
      g.beginPath();
      g.arc(x, y, 3.8, 0, Math.PI * 2);
      g.fill();
      if (i === hover || e.index === entry) {
        g.strokeStyle = e.index === entry ? LOOK.route : '#8e8e93';
        g.lineWidth = 2;
        g.beginPath();
        g.arc(x, y, e.index === entry ? 10 : 7.5, 0, Math.PI * 2);
        g.stroke();
      }
    });
    if (hover >= 0 && d.entries[hover]) {
      const e = d.entries[hover];
      g.font = '500 12px Inter, sans-serif';
      const w = g.measureText(e.name).width + 12;
      g.fillStyle = 'rgba(28,30,33,0.94)';
      g.beginPath();
      g.roundRect(X(e.at[0]) - w / 2, Y(e.at[1]) - 34, w, 22, 8);
      g.fill();
      g.fillStyle = '#ffffff';
      g.fillText(plain(e.name), X(e.at[0]) - w / 2 + 6, Y(e.at[1]) - 19);
    }
  };
  const latest = useRef(draw);
  latest.current = draw;
  /** Draw on the next frame (once, however many moves come before it). */
  const request = () => {
    if (frame.current) return;
    const go = () => {
      if (!frame.current) return;
      frame.current = 0;
      latest.current();
    };
    frame.current = requestAnimationFrame(go);
    // (a hidden window gets no frames)
    setTimeout(go, 100);
  };
  /** Moving or zooming: no new picture is started until it has stopped for a moment. */
  const moving = () => {
    busy.current = true;
    clearTimeout(settle.current);
    settle.current = setTimeout(() => {
      busy.current = !!drag.current;
      request();
    }, 150);
    request();
  };
  useEffect(() => request(), [data, hover, entry, stops.map((x) => `${x.time} ${x.name}`).join('|')]);
  useEffect(() => () => {
    clearTimeout(settle.current);
    cancelAnimationFrame(frame.current);
    job.current?.stop();
  }, []);
  useEffect(() => {
    const redraw = () => latest.current();
    const ro = new ResizeObserver(redraw);
    ro.observe(box.current);
    document.fonts?.ready.then(redraw);
    const theme = new MutationObserver(redraw);
    theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    const scale = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    scale.addEventListener('change', redraw);
    return () => {
      ro.disconnect();
      theme.disconnect();
      scale.removeEventListener('change', redraw);
    };
  }, []);

  const at = (e) => {
    const r = cv.current.getBoundingClientRect();
    return [e.clientX - r.left, e.clientY - r.top];
  };
  const entryAt = ([px, py]) => {
    const d = data;
    const v = view.current;
    if (!d?.entries || !v) return -1;
    const W = box.current.clientWidth;
    const H = box.current.clientHeight;
    let best = -1;
    let bd = 12 * 12;
    d.entries.forEach((e, i) => {
      const dx = W / 2 + (e.at[0] - v.cx) / v.mpp - px;
      const dy = H / 2 - (e.at[1] - v.cy) / v.mpp - py;
      if (dx * dx + dy * dy < bd) (bd = dx * dx + dy * dy), (best = i);
    });
    return best;
  };
  const drag = useRef(null);
  return html`<div class="routemap" ref=${box} style=${height ? `height:${height}px` : ''}
      onPointerDown=${(e) => {
        drag.current = { p: at(e), moved: 0 };
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove=${(e) => {
        const p = at(e);
        const d = drag.current;
        if (d && view.current) {
          const dx = p[0] - d.p[0];
          const dy = p[1] - d.p[1];
          d.moved += Math.abs(dx) + Math.abs(dy);
          d.p = p;
          view.current.cx -= dx * view.current.mpp;
          view.current.cy += dy * view.current.mpp;
          moving();
        } else {
          const h = entryAt(p);
          if (h !== hover) setHover(h);
        }
      }}
      onPointerUp=${(e) => {
        const d = drag.current;
        drag.current = null;
        if (d?.moved) {
          clearTimeout(settle.current);
          busy.current = false;
          request();
        }
        if (d && d.moved < 5) {
          const i = entryAt(at(e));
          if (i >= 0) onEntry?.(data.entries[i].index);
        }
      }}
      onPointerLeave=${() => setHover(-1)}
      onWheel=${(e) => {
        e.preventDefault();
        const v = view.current;
        if (!v) return;
        const [px, py] = at(e);
        const W = box.current.clientWidth;
        const H = box.current.clientHeight;
        const wx = v.cx + (px - W / 2) * v.mpp;
        const wy = v.cy - (py - H / 2) * v.mpp;
        v.mpp = Math.min(Math.max(v.mpp * (e.deltaY > 0 ? 1.15 : 1 / 1.15), 0.15), 200);
        v.cx = wx - (px - W / 2) * v.mpp;
        v.cy = wy + (py - H / 2) * v.mpp;
        moving();
      }}>
    <canvas ref=${cv}></canvas>
    ${!data && html`<div class="state"><${Spinner} /> ${t('Reading the map…')}</div>`}
    ${data?.error && html`<div class="state">${data.error}</div>`}
    ${data?.package?.credit && html`<div class="credit">${data.package.credit}</div>`}
    ${data && !data.error && html`<div class="legend">${t('{roads} roads | {stops} stops | {entries} entry points', { roads: data.roads.length, stops: data.stops.length, entries: data.entries.length })}</div>`}
  </div>`;
}
