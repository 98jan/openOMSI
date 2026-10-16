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


// The map package's surface (`d.package`, see docs/MAP_PACKAGE.md): its triangles and lines as
// Path2D objects in map metres, made once and drawn under a transform that follows the view.
const FILL_KEYS = ['water', 'green', 'built', 'grass', 'forest', 'sport', 'farm']; // by class
const FILL_ORDER = [2, 6, 3, 1, 4, 5, 0]; // under everything: built-up land first, water last

function surfacePaths(pkg) {
  if (pkg.paths) return pkg.paths;
  const fills = pkg.fills.map((f) => {
    const path = new Path2D();
    for (let i = 0; i + 5 < f.length; i += 6) {
      path.moveTo(f[i], f[i + 1]);
      path.lineTo(f[i + 2], f[i + 3]);
      path.lineTo(f[i + 4], f[i + 5]);
      path.closePath();
    }
    return path;
  });
  // a river's width tells river from stream; the railway and the borders are one path each
  const lines = { river: new Path2D(), stream: new Path2D(), rail: new Path2D(), border: new Path2D() };
  for (const l of pkg.lines) {
    const path = l.c === 0 ? (l.w >= 5 ? lines.river : lines.stream) : l.c === 1 ? lines.rail : l.c === 2 ? lines.border : null;
    if (!path) continue;
    for (let i = 0; i + 1 < l.p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, l.p[i], l.p[i + 1]);
  }
  return (pkg.paths = { fills, lines });
}

/** The surface under the roads: ground, areas, then rivers, rail and borders. */
function drawSurface(g, pkg, v, W, H) {
  const c = pkg.colors;
  const { fills, lines } = surfacePaths(pkg);
  g.save();
  // map metres (y up) onto the canvas: widths below are pixels, so they are scaled by `mpp`
  g.translate(W / 2 - v.cx / v.mpp, H / 2 + v.cy / v.mpp);
  g.scale(1 / v.mpp, -1 / v.mpp);
  g.lineCap = g.lineJoin = 'round';
  for (const k of FILL_ORDER) {
    g.fillStyle = g.strokeStyle = c[FILL_KEYS[k]];
    g.fill(fills[k]);
    g.lineWidth = 0.8 * v.mpp; // (no seams between neighbouring triangles)
    g.stroke(fills[k]);
  }
  const stroke = (path, color, px) => {
    g.strokeStyle = color;
    g.lineWidth = px * v.mpp;
    g.stroke(path);
  };
  stroke(lines.river, c.river, 6);
  stroke(lines.stream, c.river, 2.4);
  stroke(lines.rail, c.rail_edge, 3.4);
  stroke(lines.rail, c.rail, 1.6);
  g.globalAlpha = 0.7;
  stroke(lines.border, c.border, 1.2);
  g.restore();
}

// The look of the game's light map (navigator/style.rs), after Apple Maps.
const LOOK = {
  land: '#dcebcb', grass: '#dcebcb', forest: '#bcdba5', field: '#ebebcf', paved: '#edebe6', sand: '#f1e8cf', water: '#9fcff2',
  road: '#ffffff', roadEdge: '#cfccc4', foot: '#f6f4ef', footEdge: '#e1ded6', area: '#efeef0', areaEdge: '#d8d6da',
  roof: '#faf9f6', roofEdge: '#d6d1c7', shadow: 'rgba(40,58,46,0.16)', rail: '#a7a59f', tram: '#9a95a6',
  route: '#2f7cf6', routeEdge: '#1a5fcf', card: 'rgba(255,255,255,0.95)', text: '#1c1c1e', dim: '#6e6e73',
};
// land classes in the order they are laid: grass, field, sand, paved, forest, water
const LAND = [[0, 'grass'], [2, 'field'], [4, 'sand'], [3, 'paved'], [1, 'forest'], [5, 'water']];

/** The layers' triangles and outlines as Path2D objects in map metres, made once. */
function layerPaths(l) {
  if (l.paths) return l.paths;
  const tris = (t) => {
    const path = new Path2D();
    for (let i = 0; i + 5 < t.length; i += 6) {
      path.moveTo(t[i], t[i + 1]);
      path.lineTo(t[i + 2], t[i + 3]);
      path.lineTo(t[i + 4], t[i + 5]);
      path.closePath();
    }
    return path;
  };
  const poly = (p, dx = 0, dy = 0) => {
    const path = new Path2D();
    for (let i = 0; i + 1 < p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, p[i] + dx, p[i + 1] + dy);
    path.closePath();
    return path;
  };
  const houses = new Path2D();
  const shadows = new Path2D();
  for (const h of l.houses) {
    houses.addPath(poly(h.p));
    // (the shadow falls south-east, longer for a taller house)
    const k = Math.min(h.h, 40) * 0.12;
    shadows.addPath(poly(h.p, k, -k));
  }
  const rail = new Path2D();
  const tram = new Path2D();
  for (const x of l.lines) {
    const path = x.c === 0 ? rail : x.c === 1 ? tram : null;
    if (!path) continue;
    for (let i = 0; i + 1 < x.p.length; i += 2) (i ? path.lineTo : path.moveTo).call(path, x.p[i], x.p[i + 1]);
  }
  return (l.paths = { land: l.land.map(tris), paved: l.paved.map(tris), houses, shadows, rail, tram });
}

/** The ground, the paved surfaces, the tracks and the houses, under the roads' middles. */
function drawLayers(g, l, v, W, H) {
  const P = layerPaths(l);
  g.save();
  g.translate(W / 2 - v.cx / v.mpp, H / 2 + v.cy / v.mpp);
  g.scale(1 / v.mpp, -1 / v.mpp);
  g.lineCap = g.lineJoin = 'round';
  const fill = (path, color, seam = 0.8) => {
    g.fillStyle = g.strokeStyle = color;
    g.fill(path);
    g.lineWidth = seam * v.mpp; // (no seams between neighbouring triangles)
    g.stroke(path);
  };
  for (const [k, key] of LAND) if (P.land[k]) fill(P.land[k], LOOK[key]);
  const near = v.mpp < 2.5;
  // pavements and yards (near), carriageways: an edge, then the surface
  const paved = near ? [[1, 'foot', 'footEdge'], [2, 'area', 'areaEdge'], [0, 'road', 'roadEdge']] : [[2, 'area', 'areaEdge'], [0, 'road', 'roadEdge']];
  for (const [k, f, e] of paved) {
    if (!P.paved[k]) continue;
    g.strokeStyle = LOOK[e];
    g.lineWidth = 2 * v.mpp;
    g.stroke(P.paved[k]);
    fill(P.paved[k], LOOK[f]);
  }
  g.strokeStyle = LOOK.rail;
  g.lineWidth = Math.max(3.4, 3.6 / v.mpp) * v.mpp;
  g.stroke(P.rail);
  if (near) {
    g.setLineDash([9, 9]);
    g.strokeStyle = '#ffffff';
    g.lineWidth = Math.max(1.4, 1.3 / v.mpp) * v.mpp;
    g.stroke(P.rail);
    g.setLineDash([]);
  }
  g.strokeStyle = LOOK.tram;
  g.lineWidth = 1.3 * v.mpp;
  g.stroke(P.tram);
  if (v.mpp < 6) {
    g.fillStyle = LOOK.shadow;
    g.fill(P.shadows);
    g.fillStyle = LOOK.roof;
    g.strokeStyle = LOOK.roofEdge;
    g.lineWidth = 0.8 * v.mpp;
    g.fill(P.houses);
    g.stroke(P.houses);
  }
  g.restore();
}

function load(map, date, trip) {
  const key = `${map}|${date}|${trip}`;
  if (!cache.has(key)) cache.set(key, call('route_map', { map, date, trip }).then(onRoute, (e) => ({ error: e.message })));
  return cache.get(key);
}

export function RouteMap({ map, date, trip, stops = [], entry = -1, onEntry, height = 340 }) {
  const box = useRef();
  const cv = useRef();
  const [data, setData] = useState(null);
  const view = useRef(null);
  const [hover, setHover] = useState(-1);

  useEffect(() => {
    if (!map) return;
    let alive = true;
    load(map, date, trip).then((d) => {
      if (!alive) return;
      setData(d);
      view.current = null;
    });
    return () => (alive = false);
  }, [map, date, trip]);

  // the picture, drawn again whenever anything it shows changes
  const draw = () => {
    const c = cv.current;
    const d = data;
    if (!c || !d || d.error) return;
    const dpr = window.devicePixelRatio || 1;
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
      const span = Math.max((hi[0] - lo[0]) / (W - 80), (hi[1] - lo[1]) / (H - 60), 0.2);
      view.current = { cx: (lo[0] + hi[0]) / 2, cy: (lo[1] + hi[1]) / 2, mpp: span };
    }
    const v = view.current;
    const g = c.getContext('2d');
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, W, H);
    const X = (x) => W / 2 + (x - v.cx) / v.mpp;
    const Y = (y) => H / 2 - (y - v.cy) / v.mpp;
    const line = (p) => {
      g.beginPath();
      for (let i = 0; i < p.length; i += 2) (i ? g.lineTo : g.moveTo).call(g, X(p[i]), Y(p[i + 1]));
    };
    g.fillStyle = LOOK.land;
    g.fillRect(0, 0, W, H);
    if (d.package) drawSurface(g, d.package, v, W, H);
    if (d.layers) drawLayers(g, d.layers, v, W, H);
    g.lineCap = g.lineJoin = 'round';
    // the roads' middles: where a road has a paved surface it lies under it, where not it is
    // the road (its edge, then the white)
    const paved = !!d.layers && v.mpp < 2.5;
    for (let pass = 0; pass < 2; pass++) {
      for (const r of d.roads) {
        line(r.p);
        g.strokeStyle = pass ? LOOK.road : LOOK.roadEdge;
        const w = paved ? Math.max(r.w - 1.2, 1.5) : r.w;
        g.lineWidth = Math.max((w + (pass ? 0 : paved ? 0 : 1.2)) / v.mpp, pass ? (r.main ? 2.2 : 1.4) : r.main ? 3.4 : 2.6);
        g.stroke();
      }
    }
    for (const [color, wide] of [[LOOK.routeEdge, 7], [LOOK.route, 4.6]]) {
      g.strokeStyle = color;
      g.lineWidth = wide;
      for (const p of d.route) {
        line(p);
        g.stroke();
      }
    }
    // stops: white pins with the bus's blue, a card with the time and name beside
    g.font = '600 11.5px Inter, sans-serif';
    // (a label that would cover one already written goes to the left of its stop, else away)
    const taken = [];
    const free = (x, y, w) => !taken.some((r) => x < r[0] + r[2] && x + w > r[0] && y < r[1] + 20 && y + 20 > r[1]);
    for (const s of d.stops) {
      const x = X(s.at[0]);
      const y = Y(s.at[1]);
      g.fillStyle = '#ffffff';
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
        g.shadowColor = 'rgba(0,0,0,0.18)';
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
      g.fillStyle = '#ffffff';
      g.beginPath();
      g.arc(x, y, 5.5, 0, Math.PI * 2);
      g.fill();
      g.fillStyle = '#ff9f0a';
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
  useEffect(draw, [data, hover, entry, stops.map((x) => `${x.time} ${x.name}`).join('|')]);
  useEffect(() => {
    const redraw = () => latest.current();
    const ro = new ResizeObserver(redraw);
    ro.observe(box.current);
    document.fonts?.ready.then(redraw);
    const scale = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    scale.addEventListener('change', redraw);
    return () => {
      ro.disconnect();
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
          draw();
        } else {
          const h = entryAt(p);
          if (h !== hover) setHover(h);
        }
      }}
      onPointerUp=${(e) => {
        const d = drag.current;
        drag.current = null;
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
        draw();
      }}>
    <canvas ref=${cv}></canvas>
    ${!data && html`<div class="state"><${Spinner} /> ${t('Reading the map…')}</div>`}
    ${data?.error && html`<div class="state">${data.error}</div>`}
    ${data?.package?.credit && html`<div class="credit">${data.package.credit}</div>`}
    ${data && !data.error && html`<div class="legend">${t('{roads} roads | {stops} stops | {entries} entry points', { roads: data.roads.length, stops: data.stops.length, entries: data.entries.length })}</div>`}
  </div>`;
}
