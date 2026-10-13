// Injected into the planner page before its own scripts run.
// Everything the MCP tools know about the planner's internals lives here, so a
// planner update is fixed in this file rather than in the Rust host.
//
// Coordinates are the planner's own placement cells: x grows right, y grows down,
// and an entity's position is the top-left cell of its footprint.
(() => {
  const VERSION = 2;
  if (window.__enkad && window.__enkad.dispose) window.__enkad.dispose(); // dev hot reload

  // The simulation class is found by scanning loaded modules; keep them all listed.
  try { performance.setResourceTimingBufferSize(2000); } catch (e) { /* older engines */ }

  const TICKS_PER_MINUTE = 240;
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const round = (n, digits = 3) => Math.round(n * 10 ** digits) / 10 ** digits;

  // ---- planner handles -----------------------------------------------------

  // Debug handles the planner itself exposes on beta builds.
  const REQUIRED_GLOBALS = ['__area', '__glarea'];

  function area() {
    const a = typeof window.__area === 'function' ? window.__area() : null;
    if (!a) throw new Error('no plan is open in the planner');
    return a;
  }

  /** Every placed entity as {e, x, y}, ground layer first, then the pipe layer. */
  function placed(a) {
    const out = [];
    for (const layer of [a.rootLayer, a.pipeLayer]) {
      for (const [e, pos] of layer.placements) out.push({ e, x: pos.x, y: pos.y });
    }
    return out;
  }

  /** Name of the open plan, from the planner's own file store. */
  function planName() {
    try {
      const store = JSON.parse(localStorage.getItem('data'));
      return new Map(store.files).get(store.currentFileId).name;
    } catch (e) {
      return null;
    }
  }

  function counts(a) {
    const out = { building: 0, belt: 0, pipe: 0 };
    for (const { e } of placed(a)) out[kindOf(e)] = (out[kindOf(e)] || 0) + 1;
    return out;
  }

  // The planner keeps every state of a plan as a blueprint string on its undo
  // stack; the last entry is the current state.
  const currentBlueprint = (a) => a.serialized || a.undoStack.past.at(-1) || '';

  const PLANNER_MODE_DEFAULT = 0;

  const isBuilding = (e) => e.cls === 'Building';
  const kindOf = (e) => (isBuilding(e) ? 'building' : e.cls.toLowerCase());

  function footprint({ e, x, y }) {
    if (isBuilding(e)) {
      const size = e.currentSize; // carries an offset for some rotated shapes
      return { x: x + size.x, y: y + size.y, w: size.w, h: size.h };
    }
    let [x0, y0, x1, y1] = [Infinity, Infinity, -Infinity, -Infinity];
    for (const c of e.cells) {
      x0 = Math.min(x0, x + c.pos.x); x1 = Math.max(x1, x + c.pos.x);
      y0 = Math.min(y0, y + c.pos.y); y1 = Math.max(y1, y + c.pos.y);
    }
    return { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
  }

  function inRegion(box, region) {
    if (!region) return true;
    return box.x < region.x + region.w && region.x < box.x + box.w
      && box.y < region.y + region.h && region.y < box.y + box.h;
  }

  /**
   * A belt or pipe as a route "x,y>x,y>...>x,y". The first point is the cell it
   * takes items from and the last is the cell it delivers into; the points
   * between are its own first cell, corners and last cell.
   */
  function routeOf({ e, x, y }) {
    const cells = e.cells;
    const first = cells[0], last = cells[cells.length - 1];
    const points = [[x + first.pos.x - first.from.x, y + first.pos.y - first.from.y]];
    for (let i = 0; i < cells.length; i++) {
      const c = cells[i], prev = cells[i - 1];
      const corner = i === 0 || i === cells.length - 1
        || c.direction.x !== prev.direction.x || c.direction.y !== prev.direction.y;
      if (corner) points.push([x + c.pos.x, y + c.pos.y]);
    }
    points.push([x + last.pos.x + last.direction.x, y + last.pos.y + last.direction.y]);
    return points.map((p) => p.join(',')).join('>');
  }

  // ---- simulation ----------------------------------------------------------

  // The planner keeps one simulation object per open plan and exposes none of
  // them. Find the class by its method names (those survive minification), then
  // note each instance as the planner's own UI uses it.
  const SIM_METHODS = ['pathRates', 'buildingItemRates', 'buildingTransfers', 'netDepotTotals'];
  let simClass = null;
  let unhook = null;
  let sims = []; // WeakRef, so closed plans can be collected

  async function locateSimClass() {
    const urls = new Set();
    const chunk = /\/_app\/immutable\/(chunks|nodes|entry)\/[^?]+\.js$/;
    for (const entry of performance.getEntriesByType('resource')) if (chunk.test(entry.name)) urls.add(entry.name);
    for (const link of document.querySelectorAll('link[rel=modulepreload]')) if (chunk.test(link.href)) urls.add(link.href);
    for (const url of urls) {
      let module;
      try { module = await import(url); } catch (e) { continue; } // already loaded, so this is a cache hit
      for (const value of Object.values(module)) {
        if (typeof value !== 'function' || !value.prototype) continue;
        if (SIM_METHODS.every((name) => typeof value.prototype[name] === 'function')) return value;
      }
    }
    return null;
  }

  function hookSimClass(cls) {
    const proto = cls.prototype;
    const saved = [];
    let last = null;
    const note = (self) => {
      if (self === last) return;
      last = self;
      if (!sims.some((ref) => ref.deref() === self)) sims.push(new WeakRef(self));
    };
    for (const name of Object.getOwnPropertyNames(proto)) {
      if (name === 'constructor') continue;
      const d = Object.getOwnPropertyDescriptor(proto, name);
      if (!d.configurable) continue;
      if (typeof d.value === 'function') {
        const original = d.value;
        Object.defineProperty(proto, name, { ...d, value(...args) { note(this); return original.apply(this, args); } });
      } else if (d.get) {
        const original = d.get;
        Object.defineProperty(proto, name, { ...d, get() { note(this); return original.call(this); } });
      } else {
        continue;
      }
      saved.push([name, d]);
    }
    return () => { for (const [name, d] of saved) Object.defineProperty(proto, name, d); };
  }

  function knownSim(a) {
    sims = sims.filter((ref) => ref.deref());
    for (const ref of sims) {
      const s = ref.deref();
      if (s && s.graph && s.graph.area === a) return s;
    }
    return null;
  }

  async function prepareSim() {
    if (simClass) return true;
    simClass = await locateSimClass();
    if (!simClass) return false;
    unhook = hookSimClass(simClass);
    return true;
  }

  /** The simulation of the plan currently shown. */
  async function sim() {
    const a = area();
    if (!(await prepareSim())) throw new Error('simulation class not found: this planner build needs an adapter update');
    let s = knownSim(a);
    if (s) return s;
    // Not used since we hooked in. The UI reads it when the selection changes,
    // so select one more building for a moment.
    const selection = a.selectionEntities;
    const probe = placed(a).map((p) => p.e).find((e) => isBuilding(e) && !selection.isEntitySelected(e));
    if (probe) selection.add(probe);
    try {
      for (let i = 0; i < 40 && !s; i++) {
        await sleep(25);
        s = knownSim(a);
      }
    } finally {
      if (probe) selection.deselect(probe);
    }
    if (!s) throw new Error('simulation not available: the plan may be empty, or this planner build needs an adapter update');
    return s;
  }

  // Hook as soon as the planner announces itself, so the first solve is seen.
  if (typeof window.__area === 'function') {
    prepareSim();
  } else {
    let current;
    Object.defineProperty(window, '__area', {
      configurable: true,
      get: () => current,
      set(value) {
        current = value;
        prepareSim();
      },
    });
  }

  // ---- catalog -------------------------------------------------------------

  // The planner's serialised form of a plan, as its own paste and import accept it.
  const FORMAT_VERSION = 4;
  const ENTITY_TYPE = { building: 0, belt: 1, pipe: 2 };
  const POINT = { from: 1, corner: 0, to: 2 };
  const BLANK_BUILDING = {
    rotation: 0, recipeIds: [], portItems: [], mode: 0, value: 0, counter: 0, channel: 0, disabled: false, infinite: false,
  };
  const STUB_POINTS = [{ type: POINT.from, x: 0, y: -1 }, { type: POINT.corner, x: 0, y: 0 }, { type: POINT.to, x: 0, y: 1 }];

  /** Build one entity from its serialised form without putting it in the plan. */
  function materialise(a, serialised) {
    const Layer = a.rootLayer.constructor;
    const nextUid = a.nextUid;
    try {
      const layer = new Layer(8, 8, a.layers, undefined, undefined, true); // detached, as the planner's copy does
      layer.deserialize({ layerTag: a.rootLayer.tags, entities: [serialised] }, true);
      return [...layer.placements.keys()][0] || null;
    } finally {
      a.nextUid = nextUid; // scratch entities must not use up the plan's uids
    }
  }

  // Entity types are static game data, so one scan per page load is enough.
  let catalogCache = null;
  function catalog(a) {
    if (catalogCache) return catalogCache;
    const shapes = [
      ['building', { case: 'building', value: BLANK_BUILDING }],
      ['belt', { case: 'belt', value: { points: STUB_POINTS } }],
      ['pipe', { case: 'pipe', value: { points: STUB_POINTS } }],
    ];
    const types = [];
    for (let id = 0; id < 256; id++) {
      for (const [kind, entityData] of shapes) {
        let sample = null;
        try {
          sample = materialise(a, { entityId: id, x: 2, y: 2, uid: 0, type: ENTITY_TYPE[kind], entityData });
        } catch (e) {
          continue; // not an id of this kind
        }
        if (sample && sample.name) {
          types.push({ id, kind, name: sample.name, sample });
          break;
        }
      }
    }
    if (!types.length) throw new Error('could not read the building catalog: this planner build needs an adapter update');
    catalogCache = types;
    return types;
  }

  function findType(a, wanted) {
    const types = catalog(a);
    const text = String(wanted).trim().toLowerCase();
    const hit = types.find((t) => String(t.id) === text || t.name.toLowerCase() === text);
    if (hit) return hit;
    const close = types.filter((t) => t.name.toLowerCase().includes(text)).map((t) => t.name);
    throw new Error(`unknown building type "${wanted}"${close.length ? `; did you mean ${close.join(', ')}?` : ''}`);
  }

  const partsText = (parts) => parts.filter((p) => p.item).map((p) => `${p.count} ${p.item.name}`).join(' + ');

  /** A recipe of a building type, by recipe id or by the name of something it makes. */
  function findRecipe(type, wanted) {
    const recipes = type.sample.recipes || [];
    const text = String(wanted).trim().toLowerCase();
    const hits = recipes.filter((r) => String(r.numericId) === text
      || r.outcomes.some((o) => o.item && o.item.name.toLowerCase() === text));
    if (hits.length === 1) return hits[0];
    if (!hits.length) throw new Error(`${type.name} has no recipe "${wanted}"; see get_catalog for its recipes`);
    throw new Error(`"${wanted}" matches several ${type.name} recipes; use a recipe id: ${hits.map((r) => `${r.numericId} (${partsText(r.ingredients)})`).join(', ')}`);
  }

  // ---- editing -------------------------------------------------------------

  // Edits go through the planner's own operations (the ones behind its mouse and
  // keyboard handling), so its collision rules and connection logic apply.

  function entity(a, uid) {
    const e = a.entityByUid(uid);
    if (!e) throw new Error(`no entity with uid ${uid}`);
    return e;
  }

  function select(a, uids) {
    a.selectionEntities.clear();
    for (const uid of uids) a.selectionEntities.add(entity(a, uid));
  }

  const describe = (e) => {
    const box = footprint({ e, x: e.placement.x, y: e.placement.y });
    return isBuilding(e)
      ? { uid: e.uid, name: e.name, x: box.x, y: box.y, w: box.w, h: box.h, rotation: e.rotation }
      : { uid: e.uid, kind: kindOf(e), cells: e.cells.length, route: routeOf({ e, x: e.placement.x, y: e.placement.y }) };
  };

  /** Place serialised entities at the cells they name, exactly as a paste would. */
  function place(a, kindLayerTag, serialised) {
    const layerTag = (kindLayerTag & a.pipeLayer.tags) ? a.pipeLayer.tags : a.rootLayer.tags;
    const existing = new Set(a.layers.list);
    a.armSerialized({ version: FORMAT_VERSION, layers: [{ layerTag, entities: [serialised] }] });
    const moving = a.layers.list.filter((layer) => !existing.has(layer));
    if (!moving.length) throw new Error('the planner did not accept the new entity');
    // A paste follows the pointer; pin it so the entity lands on its own cells.
    for (const layer of moving) { layer.region.x = 0; layer.region.y = 0; }
    a.testIntersections();
    if (!a.confirmPlacement(false)) {
      a.cancelAllLayers();
      throw new Error('cannot place it there: the cells are occupied or outside the plan');
    }
    const placedEntities = a.selectionEntities.allEntries.map(([e]) => e);
    a.selectionEntities.clear();
    return placedEntities.map(describe);
  }

  function parseRoute(route) {
    const points = String(route).split('>').map((p) => p.split(',').map(Number));
    if (points.length < 3 || points.some((p) => p.length !== 2 || p.some((n) => !Number.isInteger(n)))) {
      throw new Error(`bad route "${route}": expected "x,y>x,y>x,y" with the source cell first and the destination cell last`);
    }
    for (let i = 1; i < points.length; i++) {
      const dx = points[i][0] - points[i - 1][0], dy = points[i][1] - points[i - 1][1];
      if ((dx !== 0) === (dy !== 0)) throw new Error(`bad route "${route}": each step must run along one axis`);
    }
    const step = (p, q) => Math.abs(p[0] - q[0]) + Math.abs(p[1] - q[1]);
    if (step(points[0], points[1]) !== 1 || step(points.at(-1), points.at(-2)) !== 1) {
      throw new Error(`bad route "${route}": the source and destination cells must touch the first and last cell of the path`);
    }
    return points;
  }

  const OPS = {
    add_building(a, op) {
      const type = findType(a, op.type);
      if (type.kind !== 'building') throw new Error(`${type.name} is a ${type.kind}: use add_path`);
      const rotation = (((op.rotation ?? 0) % 360) + 360) % 360;
      if (rotation % 90) throw new Error('rotation must be 0, 90, 180 or 270');
      const value = { ...BLANK_BUILDING, rotation, recipeIds: op.recipe == null ? [] : [findRecipe(type, op.recipe).numericId] };
      const entityData = { case: 'building', value };
      // x and y name the top-left cell of the footprint; the planner stores an anchor.
      const size = materialise(a, { entityId: type.id, x: 2, y: 2, uid: 0, type: ENTITY_TYPE.building, entityData }).currentSize;
      return place(a, type.sample.layerTag, {
        entityId: type.id, x: op.x - size.x, y: op.y - size.y, uid: 0, type: ENTITY_TYPE.building, entityData,
      });
    },

    add_path(a, op) {
      const kind = op.kind || 'belt';
      const type = catalog(a).find((t) => t.kind === kind);
      if (!type) throw new Error(`unknown path kind "${kind}": use belt or pipe`);
      const points = parseRoute(op.route);
      const [x, y] = points[1];
      const relative = points.map(([px, py], i) => ({
        type: i === 0 ? POINT.from : i === points.length - 1 ? POINT.to : POINT.corner,
        x: px - x, y: py - y,
      }));
      return place(a, type.sample.layerTag, {
        entityId: type.id, x, y, uid: 0, type: ENTITY_TYPE[kind], entityData: { case: kind, value: { points: relative } },
      });
    },

    delete(a, op) {
      select(a, op.uids);
      a.deleteSelection();
      const left = op.uids.filter((uid) => a.entityByUid(uid));
      if (left.length) throw new Error(`could not delete uid ${left.join(', ')}`);
      return { deleted: op.uids.length };
    },

    move(a, op) {
      const before = op.uids.map((uid) => { const p = entity(a, uid).placement; return [p.x, p.y]; });
      select(a, op.uids);
      a.nudgeSelection(op.dx || 0, op.dy || 0); // the planner puts everything back if the target is blocked
      a.selectionEntities.clear();
      const moved = op.uids.every((uid, i) => {
        const p = entity(a, uid).placement;
        return p.x === before[i][0] + (op.dx || 0) && p.y === before[i][1] + (op.dy || 0);
      });
      if (!moved) throw new Error('cannot move there: the cells are occupied or outside the plan');
      return op.uids.map((uid) => describe(entity(a, uid)));
    },

    rotate(a, op) {
      const before = op.uids.map((uid) => entity(a, uid).rotation);
      select(a, op.uids);
      a.rotateAllLayers(op.clockwise !== false);
      if (a.mode !== PLANNER_MODE_DEFAULT) a.cancelAllLayers(); // a blocked rotation leaves the selection lifted
      a.selectionEntities.clear();
      const turned = op.uids.some((uid, i) => !isBuilding(entity(a, uid)) || entity(a, uid).rotation !== before[i]);
      if (!turned) throw new Error('cannot rotate there: the cells are occupied or outside the plan');
      return op.uids.map((uid) => describe(entity(a, uid)));
    },

    set_recipe(a, op) {
      const e = entity(a, op.uid);
      if (!isBuilding(e)) throw new Error(`uid ${op.uid} is not a building`);
      const type = findType(a, e.numericId);
      e.state.recipes = op.recipe == null ? [] : [findRecipe(type, op.recipe)];
      return { uid: e.uid, name: e.name, recipe: recipeText(e.state.recipes) };
    },
  };

  /** Apply a list of edits as one step: all of them, or none. */
  async function edit({ ops }) {
    const a = area();
    if (a.readOnly()) throw new Error('the open plan is read-only: press Edit in the planner to work on a copy');
    if (!Array.isArray(ops) || !ops.length) throw new Error('no edits given');
    a.mode = PLANNER_MODE_DEFAULT; // drop anything the user had half-placed
    const stack = a.undoStack;
    const depth = stack.past.length;
    const start = currentBlueprint(a);
    const results = [];
    try {
      for (const [i, op] of ops.entries()) {
        if (!OPS[op.op]) throw new Error(`edit ${i + 1}: unknown op "${op.op}"`);
        try {
          results.push({ op: op.op, result: OPS[op.op](a, op) });
        } catch (e) {
          throw new Error(`edit ${i + 1} (${op.op}): ${e.message}`);
        }
      }
    } catch (e) {
      // Put the plan back as it was and leave the undo history untouched. The
      // guard tells the planner's next re-serialise not to record an undo step.
      a.mode = PLANNER_MODE_DEFAULT;
      a.selectionEntities.clear();
      a.serializeGuard = true;
      a.deserialize(start);
      await nextSerialise(a);
      a.serializeGuard = false;
      stack.past.length = Math.min(stack.past.length, depth);
      throw new Error(`${e.message}. No edits were applied.`);
    }
    // Let the planner record the result itself, then make sure the whole batch
    // is exactly one undo step.
    a.serializeGuard = false;
    await nextSerialise(a);
    stack.past.splice(depth, Math.max(0, stack.past.length - depth - 1));
    return { applied: results.length, results, counts: counts(a), undoDepth: stack.past.length };
  }

  /**
   * Resolves once the planner has next re-serialised the plan, which it does a
   * moment after any change (that is also what saves the plan and feeds undo).
   */
  function nextSerialise(a, timeoutMs = 1500) {
    return new Promise((resolve) => {
      const original = a.serialize;
      const timer = setTimeout(() => { delete a.serialize; resolve(false); }, timeoutMs);
      a.serialize = function (...args) {
        delete a.serialize; // back to the planner's own method
        clearTimeout(timer);
        const result = original.apply(this, args);
        resolve(true);
        return result;
      };
    });
  }

  // ---- formatting ----------------------------------------------------------

  const perMinute = (ratePerTick) => round(ratePerTick * TICKS_PER_MINUTE);
  const mark = (r) => (r.converged === false ? '~' : ''); // "~" = rate has not settled
  /** "Amethyst Ore 30, Water 12.5~" (per minute). */
  const rateList = (rates, scale) => rates
    .map((r) => `${r.item ? r.item.name : '#' + r.itemId} ${round(r.rate * scale)}${mark(r)}`)
    .join(', ');

  function itemNames(a) {
    const names = new Map();
    for (const { e } of placed(a)) {
      if (!isBuilding(e)) continue;
      for (const recipe of e.recipes || []) {
        for (const part of [...(recipe.ingredients || []), ...(recipe.outcomes || [])]) {
          if (part && part.item) names.set(part.item.numericId, part.item.name);
        }
      }
    }
    return names;
  }

  const recipeText = (recipes) => recipes
    .filter(Boolean)
    .map((r) => r.outcomes.filter((o) => o.item).map((o) => o.item.name).join('+'))
    .join(' | ');

  // ---- API -----------------------------------------------------------------

  const api = {
    health() {
      const globals = Object.fromEntries(REQUIRED_GLOBALS.map((k) => [k, typeof window[k]]));
      return {
        adapter: VERSION,
        url: location.href,
        ready: document.readyState,
        globals,
        simClass: !!simClass,
        ok: REQUIRED_GLOBALS.every((k) => typeof window[k] !== 'undefined'),
      };
    },

    async overview() {
      const a = area();
      const bounds = a.contentBounds;
      const out = {
        plan: {
          name: planName(),
          width: a.safeW, height: a.safeH,
          contentBounds: { x: bounds.x, y: bounds.y, w: bounds.w, h: bounds.h },
          counts: counts(a),
          readOnly: !!a.readOnly(),
          undoDepth: a.undoStack.past.length,
        },
      };
      try {
        const s = await sim();
        out.sim = {
          tick: s.targetTick, ticks: s.tickCount, ticksPerMinute: TICKS_PER_MINUTE,
          solving: s.solveProgress != null || !!s.solvePending, stale: !!s.stale,
        };
      } catch (e) {
        out.sim = { error: e.message };
      }
      return out;
    },

    /** Buildings, belts and pipes, optionally limited to a region {x, y, w, h}. */
    async layout({ region } = {}) {
      const a = area();
      const s = await sim().catch(() => null);
      const buildings = [], paths = [];
      for (const p of placed(a)) {
        const box = footprint(p);
        if (!inRegion(box, region)) continue;
        if (isBuilding(p.e)) {
          const recipe = s ? recipeText(s.getBuildingRecipes(p.e) || []) : '';
          buildings.push([p.e.uid, p.e.name, box.x, box.y, box.w, box.h, p.e.rotation, recipe]);
        } else {
          paths.push([p.e.uid, kindOf(p.e), p.e.cells.length, routeOf(p)]);
        }
      }
      return {
        buildings: { columns: ['uid', 'name', 'x', 'y', 'w', 'h', 'rotation', 'recipe'], rows: buildings },
        paths: { columns: ['uid', 'kind', 'cells', 'route'], rows: paths },
        note: 'a route runs from the cell it takes items from, through its own corner cells, to the cell it delivers into',
      };
    },

    /** Who feeds whom, with the rate per minute on each connection. */
    async graph({ uids } = {}) {
      const s = await sim();
      const only = uids && new Set(uids);
      // The simulation keys an entity as uid * 2; a bridge's second lane is uid * 2 + 1.
      const uidOfKey = (key) => key >> 1;
      const rows = [];
      for (const [from, edges] of s.graph.outgoing) {
        const out = s.buildingEdgeRates(from).filter((r) => r.dir === 1);
        for (const edge of edges) {
          const to = edge.bPort.parent;
          if (only && !only.has(from.uid) && !only.has(to.uid)) continue;
          const via = edge.entities.map((e) => e.uid);
          // Two belts between the same pair share a peer, so prefer the belt's own rate.
          const rates = edge.entities.length
            ? rateList(s.pathRates(edge.entities[0]), 1)
            : rateList(out.filter((r) => uidOfKey(r.peer) === to.uid), TICKS_PER_MINUTE);
          rows.push([from.uid, from.name, edge.aPort.index, to.uid, to.name, edge.bPort.index, via.join(' '), rates]);
        }
      }
      return {
        columns: ['from', 'fromName', 'outPort', 'to', 'toName', 'inPort', 'via', 'perMinute'],
        rows,
        note: '"~" after a rate means it has not settled',
      };
    },

    /** Per-building input and output rates, starved ports, stalls and depot totals. */
    async flows({ uids, region } = {}) {
      const a = area(), s = await sim();
      const only = uids && new Set(uids);
      const stalled = new Map(s.stalledAt(s.targetTick).map((x) => [x.building, x.clogged ? 'clogged' : 'idle']));
      const buildings = [];
      for (const p of placed(a)) {
        if (!isBuilding(p.e) || (only && !only.has(p.e.uid)) || !inRegion(footprint(p), region)) continue;
        const rates = s.buildingItemRates(p.e);
        if (!rates.inputs.length && !rates.outputs.length && !stalled.has(p.e)) continue;
        buildings.push([
          p.e.uid, p.e.name,
          rateList(rates.inputs, TICKS_PER_MINUTE), rateList(rates.outputs, TICKS_PER_MINUTE),
          stalled.get(p.e) || '',
        ]);
      }
      const marks = [];
      for (const [port, m] of s.portMarks) {
        const owner = port.parent;
        if (only && !only.has(owner.uid)) continue;
        marks.push([owner.uid, owner.name, port.output ? 'out' : 'in', port.index, m.kind, m.wins]);
      }
      const depot = (which) => rateList(s.netDepotTotals(which), TICKS_PER_MINUTE);
      return {
        tick: s.targetTick, ticks: s.tickCount,
        solving: s.solveProgress != null || !!s.solvePending,
        buildings: { columns: ['uid', 'name', 'inPerMinute', 'outPerMinute', 'stalled'], rows: buildings },
        portMarks: { columns: ['uid', 'name', 'side', 'port', 'kind', 'wins'], rows: marks },
        depot: { producedPerMinute: depot('produced'), consumedPerMinute: depot('consumed') },
      };
    },

    /** Items moved in and out of buildings over time, summed per window of ticks. */
    async history({ uids, from = 0, to, window: size = 1200 }) {
      const a = area(), s = await sim();
      const names = itemNames(a);
      const end = Math.min(to ?? s.tickCount, s.tickCount);
      const rows = [];
      for (const uid of uids) {
        const e = a.entityByUid(uid);
        if (!e) throw new Error(`no entity with uid ${uid}`);
        const buckets = new Map(); // "window|item" -> [in, out]
        for (const t of s.buildingTransfers(e, from, end) || []) {
          const key = `${Math.floor((t.tick - from) / size)}|${t.itemId}`;
          const sums = buckets.get(key) || [0, 0];
          sums[t.kind === 0 ? 0 : 1] += t.count;
          buckets.set(key, sums);
        }
        for (const [key, [inCount, outCount]] of buckets) {
          const [w, itemId] = key.split('|').map(Number);
          rows.push([uid, from + w * size, names.get(itemId) || '#' + itemId, inCount, outCount]);
        }
      }
      rows.sort((x, y) => x[0] - y[0] || x[1] - y[1]);
      return {
        from, to: end, window: size, ticksPerMinute: TICKS_PER_MINUTE,
        columns: ['uid', 'windowStartTick', 'item', 'in', 'out'],
        rows,
      };
    },

    /** Building and path types that can be placed, or one type's recipes. */
    catalog({ type } = {}) {
      const a = area();
      if (type != null) {
        const t = findType(a, type);
        return {
          id: t.id, name: t.name,
          recipes: {
            columns: ['recipeId', 'inputs', 'outputs'],
            rows: (t.sample.recipes || []).map((r) => [r.numericId, partsText(r.ingredients), partsText(r.outcomes)]),
          },
        };
      }
      return {
        columns: ['id', 'name', 'kind', 'layer', 'w', 'h', 'inputs', 'outputs', 'recipes'],
        rows: catalog(a).map((t) => {
          const e = t.sample, building = t.kind === 'building';
          const ports = building ? e.ports : [];
          return [
            t.id, t.name, t.kind, (e.layerTag & a.pipeLayer.tags) ? 'pipe' : 'ground',
            building ? e.currentSize.w : 1, building ? e.currentSize.h : 1,
            ports.filter((p) => !p.output).length, ports.filter((p) => p.output).length,
            building ? (e.recipes || []).length : 0,
          ];
        }),
      };
    },

    edit,

    /** The open plan as the planner's own blueprint string (what its paste and import accept). */
    exportBlueprint() {
      const a = area();
      const blueprint = currentBlueprint(a);
      if (!blueprint) throw new Error('the planner has not serialised this plan yet');
      return {
        plan: planName(), width: a.safeW, height: a.safeH, counts: counts(a),
        time: new Date().toISOString(),
        blueprint,
      };
    },

    /** Replace the open plan with a blueprint string, as one step the user can undo. */
    async importBlueprint({ blueprint }) {
      const a = area();
      if (a.readOnly()) throw new Error('the open plan is read-only: press Edit in the planner to work on a copy');
      const stack = a.undoStack;
      const depth = stack.past.length;
      a.mode = PLANNER_MODE_DEFAULT;
      a.selectionEntities.clear();
      a.deserialize(blueprint);
      // deserialize records the string as an undo step, and the planner then
      // re-serialises the loaded plan and records that too. Keep only the second.
      if (stack.past.length > depth) stack.past.pop();
      for (let i = 0; i < 80 && stack.past.length <= depth; i++) await sleep(25);
      if (stack.past.length <= depth) throw new Error('not a readable blueprint string; the plan was left unchanged');
      return { plan: planName(), width: a.safeW, height: a.safeH, counts: counts(a), undoDepth: stack.past.length };
    },
  };

  window.__enkad = {
    adapter: VERSION,
    api,
    /** Entry point for the host: resolves to the API result or rejects with a plain message. */
    async call(name, args) {
      if (typeof api[name] !== 'function') throw new Error(`adapter has no "${name}"`);
      return api[name](args || {});
    },
    dispose() {
      if (unhook) unhook();
      const d = Object.getOwnPropertyDescriptor(window, '__area');
      if (d && d.get) {
        // Put the planner's handle back as a plain property.
        const value = window.__area;
        delete window.__area;
        if (value !== undefined) window.__area = value;
      }
    },
  };
})();
