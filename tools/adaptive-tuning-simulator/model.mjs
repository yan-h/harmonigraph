// Experimental policy, deliberately independent of the production plugin.
export const AXES = [1200 * Math.log2(3 / 2), 1200 * Math.log2(5 / 4), 1200 * Math.log2(7 / 4)];
export const DEFAULTS = Object.freeze({
  radius: 3, axes: 2, memory: 6, harmonic: 6, pitchScale: 20,
  released: 0.1, recency: 0.7, registerFloor: 0.4, registerFalloff: 0.8,
  tolerance: 0.5, silence: 0, resetStop: false, resetLoop: false,
});
export const keyOf = n => n.join(',');
export const latticeCents = n => 4800 + n.reduce((sum, x, i) => sum + x * AXES[i], 0);
export const distance = (a, b) => a.reduce((sum, x, i) => sum + Math.abs(x - b[i]), 0);
export const mod = (x, n) => ((x % n) + n) % n;
export function nodeName(n) {
  const diatonic = 4 * n[0] + 2 * n[1] + 6 * n[2], degree = mod(diatonic, 7);
  const nominal = 7 * n[0] + 4 * n[1] + 10 * n[2];
  const alteration = nominal - (12 * Math.floor(diatonic / 7) + [0, 2, 4, 5, 7, 9, 11][degree]);
  const symbol = alteration < 0 ? '♭' : '♯', count = Math.abs(alteration);
  return 'CDEFGAB'[degree] + (count > 2 ? `${symbol}${count}` : symbol.repeat(count));
}
export function pitchName(cents) {
  const midi = Math.round(cents / 100);
  const centsOff = cents - midi * 100;
  const name = ['C', 'C♯', 'D', 'E♭', 'E', 'F', 'F♯', 'G', 'A♭', 'A', 'B♭', 'B'][mod(midi, 12)];
  return `${name}${Math.floor(midi / 12) - 1}${Math.abs(centsOff) < 0.005 ? '' : ` ${centsOff >= 0 ? '+' : ''}${centsOff.toFixed(2)}¢`}`;
}
export function parsePitch(text) {
  const value = String(text).trim().replaceAll('♯', '#').replaceAll('♭', 'b');
  const numeric = value.match(/^(-?\d+(?:\.\d+)?)(c|m)$/i);
  if (numeric) return Number(numeric[1]) * (numeric[2].toLowerCase() === 'm' ? 100 : 1);
  const match = value.match(/^([A-Ga-g])([#b]?)(-?\d+)(?:([+-]\d+(?:\.\d+)?)c?)?$/);
  if (!match) throw new Error(`Unknown pitch “${text}”. Use E5, Eb3, E5+7.82c, 7607.82c or 76.0782m.`);
  return ((Number(match[3]) + 1) * 12 + { C: 0, D: 2, E: 4, F: 5, G: 7, A: 9, B: 11 }[match[1].toUpperCase()]
    + (match[2] === '#' ? 1 : match[2] === 'b' ? -1 : 0)) * 100 + Number(match[4] || 0);
}
function validateSettings(raw) {
  const s = { ...DEFAULTS, ...raw };
  const bounds = { radius: [1, 5], axes: [1, 3], memory: [0, 24], harmonic: [0, 20], pitchScale: [1, 100],
    released: [0, 1], recency: [0, 1], registerFloor: [0.01, 1], registerFalloff: [0, 4], tolerance: [0, 20], silence: [0, 120] };
  for (const [name, [lo, hi]] of Object.entries(bounds)) {
    if (!Number.isFinite(s[name]) || s[name] < lo || s[name] > hi) throw new Error(`Invalid ${name}: expected ${lo}–${hi}.`);
  }
  for (const name of ['radius', 'axes', 'memory']) if (!Number.isInteger(s[name])) throw new Error(`${name} must be an integer.`);
  return s;
}
function compareNodes(a, b) {
  // Relative complexity tie-break is independent of absolute register.
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

export class Simulator {
  constructor(settings = {}) {
    this.settings = validateSettings(settings);
    this.reset();
  }
  reset() {
    this.held = new Map(); this.recent = []; this.reference = 0; this.time = 0;
    this.serial = 0; this.lastRelease = null; this.history = []; this.last = null;
  }
  clearContext() {
    // A reset never changes the frozen correction of voices still sounding.
    this.recent = []; this.reference = 0; this.last = null;
  }
  context() {
    const entries = [];
    // Same-register repetitions refresh one contribution; voices remain separate
    // for release and expression. Octave duplicates retain their own registers.
    const held = [...this.held.values()].sort((a, b) => b.stamp - a.stamp);
    for (const voice of held) {
      if (!entries.some(e => Math.abs(e.output - voice.output) <= this.settings.tolerance)) {
        entries.push({ ...voice, weight: 1, status: 'held' });
      }
    }
    let rank = 0;
    for (const entry of this.recent) {
      if (entries.some(e => Math.abs(e.output - entry.output) <= this.settings.tolerance)) continue;
      const weight = this.settings.released * this.settings.recency ** rank++;
      if (weight > 0) entries.push({ ...entry, weight, status: 'released' });
    }
    // An empty phrase has an explicit origin reference, not a stale per-key map.
    if (!entries.length) entries.push({ node: [0, 0, 0], output: 4800, input: 4800, weight: 1, status: 'origin', id: 'origin' });
    return entries;
  }
  candidates(context = this.context()) {
    const { radius, axes } = this.settings;
    const result = new Map();
    for (const anchor of context) {
      for (let t = -radius; t <= radius; t++) {
        for (let f = axes >= 2 ? -radius : 0; f <= (axes >= 2 ? radius : 0); f++) {
          for (let s = axes >= 3 ? -radius : 0; s <= (axes >= 3 ? radius : 0); s++) {
            if (Math.abs(t) + Math.abs(f) + Math.abs(s) > radius) continue;
            const node = anchor.node.map((x, i) => x + [t, f, s][i]);
            result.set(keyOf(node), node);
          }
        }
      }
    }
    return [...result.values()].sort(compareNodes);
  }
  harmonicCost(node, output, context) {
    let total = 0, weightSum = 0;
    const { registerFloor: floor, registerFalloff: falloff } = this.settings;
    for (const voice of context) {
      const register = floor + (1 - floor) * Math.exp(-falloff * Math.abs(output - voice.output) / 1200);
      const weight = voice.weight * register;
      total += weight * distance(node, voice.node);
      weightSum += weight;
    }
    return total / weightSum;
  }
  evaluate(input) {
    if (!Number.isFinite(input)) throw new Error('Input pitch must be finite.');
    const context = this.context(), target = input + this.reference;
    const candidates = this.candidates(context).map(node => {
      const base = latticeCents(node);
      const octave = Math.floor((target - base) / 1200 + 0.5);
      const output = base + 1200 * octave;
      const error = output - target;
      const pitchCost = (error / this.settings.pitchScale) ** 2;
      const harmonicDistance = this.harmonicCost(node, output, context);
      const harmonicCost = this.settings.harmonic * harmonicDistance;
      return { node, octave, output, error, pitchCost, harmonicCost, harmonicDistance, score: pitchCost + harmonicCost };
    }).sort((a, b) => a.score - b.score || compareNodes(a.node, b.node));
    return { input, target, reference: this.reference, context, candidates, winner: candidates[0] };
  }
  on(input, id = `note-${this.serial + 1}`) {
    if (this.held.has(id)) this.off(id);
    const result = this.evaluate(input);
    const voice = { id, node: [...result.winner.node], input, output: result.winner.output,
      correction: result.winner.output - input, bend: 0, stamp: ++this.serial };
    this.recent = this.recent.filter(e => Math.abs(e.output - voice.output) > this.settings.tolerance);
    this.held.set(id, voice);
    // The simplest moving reference: last onset's full output-minus-input
    // correction. Never fold this number modulo 1200: long journeys cross octaves.
    this.reference = voice.correction;
    this.lastRelease = null;
    this.last = result;
    this.history.push({ ...voice, node: [...voice.node], time: this.time });
    return result;
  }
  seed(entries) {
    for (const entry of entries) {
      const input = typeof entry.input === 'string' ? parsePitch(entry.input) : entry.input;
      const base = latticeCents(entry.node);
      const output = entry.output ?? base + 1200 * Math.floor((input - base) / 1200 + 0.5);
      if (!Number.isFinite(input) || !Number.isFinite(output) || entry.node.length !== 3 || !entry.node.every(Number.isInteger)) throw new Error('Invalid fixture seed.');
      this.held.set(entry.id, { ...entry, input, output, correction: output - input, bend: 0, stamp: ++this.serial });
    }
  }
  off(id) {
    const voice = this.held.get(id);
    if (!voice) throw new Error(`No sounding note named “${id}”.`);
    this.held.delete(id);
    const entry = { ...voice, stamp: ++this.serial };
    this.recent = [entry, ...this.recent.filter(e => Math.abs(e.output - voice.output) > this.settings.tolerance)].slice(0, this.settings.memory);
    if (!this.held.size) this.lastRelease = this.time;
  }
  allOff() { for (const id of [...this.held.keys()]) this.off(id); }
  bend(id, cents) {
    if (!this.held.has(id) || !Number.isFinite(cents)) throw new Error('Bend requires a sounding note and finite cents.');
    this.held.get(id).bend = cents;
  }
  wait(seconds) {
    if (!Number.isFinite(seconds) || seconds < 0) throw new Error('Wait must be a nonnegative duration.');
    this.time += seconds;
    if (!this.held.size && this.settings.silence > 0 && this.lastRelease !== null && this.time - this.lastRelease >= this.settings.silence) {
      this.clearContext(); this.lastRelease = null;
    }
  }
  transport(event) {
    if (!['stop', 'start', 'loop'].includes(event)) throw new Error('Unknown transport event.');
    if (event === 'stop') this.allOff();
    if ((event === 'stop' && this.settings.resetStop) || (event === 'loop' && this.settings.resetLoop)) this.clearContext();
  }
  event(event) {
    if (event.type === 'on') return this.on(event.pitch, event.id);
    if (event.type === 'off') return event.id === '*' ? this.allOff() : this.off(event.id);
    if (event.type === 'wait') return this.wait(event.seconds);
    if (event.type === 'bend') return this.bend(event.id, event.cents);
    if (event.type === 'reset') { this.allOff(); this.clearContext(); return; }
    return this.transport(event.type);
  }
  reachability(low, high) {
    if (!Number.isFinite(low) || !Number.isFinite(high) || high <= low || high - low > 12000) throw new Error('Choose an ascending input range of at most ten octaves.');
    const context = this.context(), nodes = this.candidates(context);
    const scale2 = this.settings.pitchScale ** 2;
    const pieces = [];
    // Register weights use each candidate's realized output, so harmonic costs
    // are constant within an octave realization. All pitch parabolas have equal
    // curvature. Their pairwise differences are linear: compute winner intervals
    // analytically, instead of claiming a coarse pitch scan is exhaustive.
    for (const node of nodes) {
      const base = latticeCents(node);
      const first = Math.ceil((low + this.reference - 600 - base) / 1200);
      const last = Math.floor((high + this.reference + 600 - base) / 1200);
      for (let octave = first; octave <= last; octave++) {
        const output = base + 1200 * octave, center = output - this.reference;
        const lo = Math.max(low, center - 600), hi = Math.min(high, center + 600);
        if (hi <= lo) continue;
        pieces.push({ node, octave, output, center, lo, hi,
          cost: this.settings.harmonic * this.harmonicCost(node, output, context) });
      }
    }
    const bands = [];
    for (const a of pieces) {
      let ranges = [[a.lo, a.hi]];
      for (const b of pieces) {
        if (a === b || b.hi <= a.lo || b.lo >= a.hi) continue;
        // score(a)-score(b) = slope * input + intercept.
        const slope = 2 * (b.center - a.center) / scale2;
        const intercept = (a.center - b.center) * (a.center + b.center) / scale2 + a.cost - b.cost;
        let cutLo = b.lo, cutHi = b.hi;
        if (Math.abs(slope) < 1e-12) {
          if (intercept < -1e-10 || (Math.abs(intercept) <= 1e-10 && compareNodes(a.node, b.node) <= 0)) continue;
        } else if (slope > 0) cutLo = Math.max(cutLo, -intercept / slope);
        else cutHi = Math.min(cutHi, -intercept / slope);
        if (cutHi <= cutLo) continue;
        const next = [];
        for (const [lo, hi] of ranges) {
          if (cutHi <= lo || cutLo >= hi) next.push([lo, hi]);
          else {
            if (cutLo > lo) next.push([lo, Math.min(hi, cutLo)]);
            if (cutHi < hi) next.push([Math.max(lo, cutHi), hi]);
          }
        }
        ranges = next;
        if (!ranges.length) break;
      }
      for (const [lo, hi] of ranges) if (hi > lo) bands.push({ low: lo, high: hi, node: a.node, output: a.output });
    }
    bands.sort((a, b) => a.low - b.low || compareNodes(a.node, b.node));
    // Isolated ties and range endpoints can select a node with no positive-width
    // winning interval. Include their actual deterministic winners as point hits.
    const boundaries = new Set([low, high, ...bands.flatMap(b => [b.low, b.high]), ...pieces.flatMap(p => [p.lo, p.hi])]);
    const points = [...boundaries].map(input => ({ input, ...this.evaluate(input).winner }));
    return { low, high, bands, points, keys: new Set([...bands, ...points].map(b => keyOf(b.node))), nodes };
  }
}

export function parseProgram(text) {
  const events = [];
  for (const [index, line] of text.split('\n').entries()) {
    const trimmed = line.split('//')[0].trim();
    if (!trimmed) continue;
    const [command, a, b, extra] = trimmed.split(/\s+/);
    try {
      let event;
      if (command === 'on' && a && b && !extra) event = { type: 'on', pitch: parsePitch(a), id: b };
      else if (command === 'off' && a && !b) event = { type: 'off', id: a };
      else if (command === 'wait' && a && !b && Number.isFinite(Number(a)) && Number(a) >= 0) event = { type: 'wait', seconds: Number(a) };
      else if (command === 'bend' && a && b && !extra && Number.isFinite(Number(b))) event = { type: 'bend', id: a, cents: Number(b) };
      else if (['stop', 'start', 'loop', 'reset'].includes(command) && !a) event = { type: command };
      else throw new Error('Use on <pitch> <id>, off <id|*>, wait <seconds>, bend <id> <cents>, stop, start, loop or reset.');
      events.push({ ...event, line: index + 1, text: trimmed });
    } catch (error) { throw new Error(`Line ${index + 1}: ${error.message}`); }
  }
  if (events.length > 10000) throw new Error('Limit an experiment to 10,000 events.');
  return events;
}
