import { DEFAULTS, Simulator, parseProgram, parsePitch, pitchName, nodeName, keyOf, latticeCents } from './model.mjs';
import { EXAMPLES, runExample } from './examples.mjs';

const $ = id => document.getElementById(id);
const NS = 'http://www.w3.org/2000/svg';
const svg = (tag, attrs = {}, text) => {
  const el = document.createElementNS(NS, tag);
  for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value);
  if (text !== undefined) el.textContent = text;
  return el;
};
const element = (tag, text, className) => {
  const el = document.createElement(tag); if (text !== undefined) el.textContent = text;
  if (className) el.className = className; return el;
};
const signed = (n, digits = 2) => `${n >= 0 ? '+' : ''}${n.toFixed(digits)}`;
let example = EXAMPLES[0], settings = { ...DEFAULTS }, events = [], cursor = 0, sim = new Simulator();
let seed = [], selected = null, reach = null, reachWorker = null, timer = null;
let reachLow = 3600, reachHigh = 9600, reachRequest = 0, custom = false;
let audio = null, sounds = new Map();

const CONTROLS = [
  ['harmonic', 'Harmonic preference', 0, 12, 0.1, 'Higher values favour harmonic proximity over pitch accuracy.', 'main-controls'],
  ['pitchScale', 'Pitch flexibility', 1, 60, 1, 'Cents of pitch error that cost one point. Lower = more precise control.', 'main-controls'],
  ['registerFloor', 'Distant-note influence', 0.05, 1, 0.05, 'Minimum retained register weight. 1 makes scoring register-blind.', 'main-controls'],
  ['registerFalloff', 'Register sensitivity', 0, 3, 0.1, 'How quickly additional influence falls with octave separation.', 'main-controls'],
  ['released', 'Released-note influence', 0, 1, 0.05, 'Held notes weigh 1. Released notes begin at this weight.', 'main-controls'],
  ['recency', 'Memory carry-over', 0, 1, 0.05, 'Each older released contribution retains this fraction.', 'main-controls'],
  ['radius', 'Neighbourhood radius', 1, 5, 1, 'Maximum lattice steps from any contributing context note.', 'extra-controls'],
  ['memory', 'Released memory capacity', 0, 24, 1, 'Temporary limit on released entries. Held notes are retained.', 'extra-controls'],
  ['tolerance', 'Same-note tolerance', 0, 20, 0.1, 'Cents of absolute onset-pitch difference that refresh one memory entry.', 'extra-controls'],
];
for (const [name, label, min, max, step, help, container] of CONTROLS) {
  const row = element('label', undefined, 'control'); row.htmlFor = `param-${name}`;
  const heading = element('span', undefined, 'control-heading'); heading.append(element('span', label));
  const output = element('output'); output.id = `value-${name}`; heading.append(output);
  const input = element('input'); Object.assign(input, { type: 'range', min, max, step, id: `param-${name}` });
  row.append(heading, input, element('p', help)); $(container).append(row);
  input.addEventListener('input', () => { output.textContent = formatControl(name, Number(input.value)); });
  input.addEventListener('change', () => guard(() => changeSettings({ [name]: Number(input.value) })));
}
function formatControl(name, value) { return `${Number.isInteger(value) ? value : value.toFixed(2)}${['pitchScale', 'tolerance'].includes(name) ? ' ¢' : ''}`; }
function syncControls() {
  for (const [name] of CONTROLS) { $(`param-${name}`).value = settings[name]; $(`value-${name}`).textContent = formatControl(name, settings[name]); }
  $('axes').value = settings.axes; $('silence').value = settings.silence;
  $('reset-stop').checked = settings.resetStop; $('reset-loop').checked = settings.resetLoop;
}
function guard(fn) {
  try { $('error').hidden = true; return fn(); }
  catch (error) { stopPlayback(); $('error').textContent = error.message; $('error').hidden = false; }
}
function freshState(newSettings, nextCursor) {
  const next = new Simulator(newSettings); next.seed(seed);
  for (let i = 0; i < nextCursor; i++) next.event(events[i]);
  return next;
}
function changeSettings(change) {
  const nextSettings = { ...settings, ...change };
  const next = freshState(nextSettings, cursor);
  settings = nextSettings; sim = next; selected = null;
  syncControls(); update(true);
}
function loadExample(id) {
  stopPlayback();
  example = EXAMPLES.find(e => e.id === id) || EXAMPLES[0];
  settings = { ...DEFAULTS, ...example.settings }; seed = structuredClone(example.seed || []);
  events = parseProgram(example.program); cursor = 0; custom = false;
  sim = freshState(settings, 0); selected = null;
  $('example').value = example.id; $('description').textContent = example.description;
  $('expected').textContent = `Target: ${example.expected}`;
  $('program').value = example.program; $('pitch').value = pitchName(events.find(e => e.type === 'on')?.pitch ?? 5200).split(' ')[0]; $('fine').value = 0;
  syncControls(); update(true);
}
function seek(nextCursor) {
  nextCursor = Math.max(0, Math.min(events.length, nextCursor));
  const next = freshState(settings, nextCursor);
  sim = next; cursor = nextCursor; selected = null; update(true);
}
function step() { if (cursor >= events.length) { stopPlayback(); return; } seek(cursor + 1); }
function stopPlayback() { if (timer) clearInterval(timer); timer = null; $('play').textContent = 'Play'; }
function togglePlay() {
  if (timer) { stopPlayback(); return; }
  if (cursor === events.length) seek(0);
  $('play').textContent = 'Pause';
  timer = setInterval(() => guard(step), Number($('speed').value));
}
function appendEvent(event, text) {
  stopPlayback();
  const candidate = { ...event, text };
  const old = events;
  events = [...events.slice(0, cursor), candidate];
  try { const next = freshState(settings, events.length); sim = next; }
  catch (error) { events = old; throw error; }
  cursor = events.length; custom = true; $('program').value = events.map(e => e.text).join('\n');
  $('expected').textContent = 'Custom continuation · fixture target is not evaluated for edited sequences.';
  selected = null; update(true);
}
function probeInput() { return parsePitch($('pitch').value) + Number($('fine').value); }
function update(recalculateReach = false) {
  const next = sim.evaluate(probeInput());
  $('next-output').textContent = `${nodeName(next.winner.node)} · ${pitchName(next.winner.output)}`;
  $('next-correction').textContent = `${signed(next.winner.output - next.input)} ¢`;
  $('reference').textContent = `${signed(sim.reference)} ¢`;
  $('step-count').textContent = `${cursor} / ${events.length}`;
  $('scrub').max = events.length; $('scrub').value = cursor;
  $('step').disabled = cursor >= events.length; $('finish').disabled = cursor >= events.length;
  const lastEvent = events[cursor - 1];
  $('event-label').textContent = lastEvent ? `${lastEvent.text} · virtual time ${sim.time.toFixed(2)} s` : 'Starting context · reference 0.00 ¢';
  $('history-count').textContent = `${sim.history.length} assigned onsets`;
  if (!custom && cursor === events.length && events.length) {
    const matches = example.check ? example.check(sim) : keyOf(sim.history.at(-1)?.node || []) === keyOf(example.expectedNode);
    $('expected').textContent = `${matches ? 'Target reached:' : 'Target not reached at these settings. Expected:'} ${example.expected}`;
    $('expected').className = matches ? 'pass' : 'fail';
  } else $('expected').className = 'muted';
  renderScores(next); renderContext(); renderJourney();
  if (recalculateReach) requestReach();
  renderLattice(next); syncSound();
}
function requestReach() {
  reachWorker?.terminate(); reachWorker = null; reach = null;
  const generation = ++reachRequest;
  $('reach-status').textContent = 'Computing full winner intervals…';
  const worker = new Worker(new URL('./reach-worker.mjs', import.meta.url), { type: 'module' }); reachWorker = worker;
  worker.onmessage = ({ data }) => {
    if (generation !== reachRequest) return;
    worker.terminate(); reachWorker = null;
    if (data.error) { $('reach-status').textContent = data.error; return; }
    reach = data.result;
    $('reach-status').textContent = `${reach.keys.size} reachable / ${reach.nodes.length} eligible · ${Math.round(data.milliseconds)} ms`;
    guard(() => { renderLattice(sim.evaluate(probeInput())); renderSelected(); });
  };
  worker.onerror = () => { if (generation === reachRequest) $('reach-status').textContent = 'Reachability failed. Serve this folder over localhost; see README.'; worker.terminate(); };
  worker.postMessage({ settings, context: sim.context(), reference: sim.reference, low: reachLow, high: reachHigh });
}
function renderScores(next) {
  const view = $('score-mode').value === 'last' && sim.last ? sim.last : next;
  $('decision-caption').textContent = `${$('score-mode').value === 'last' && sim.last ? 'Frozen decision for' : 'Preview for'} ${pitchName(view.input)}. Target ${pitchName(view.target)} after reference ${signed(view.reference)} ¢. Lower total wins; first 12 of ${view.candidates.length} candidates.`;
  const body = $('scores'); body.replaceChildren();
  for (const [i, c] of view.candidates.slice(0, 12).entries()) {
    const row = element('tr'); if (i === 0) row.classList.add('winner');
    if (selected && keyOf(c.node) === keyOf(selected)) row.classList.add('selected-row');
    for (const value of [`${nodeName(c.node)} [${c.node.join(', ')}]${i === 0 ? ' · chosen' : ''}`, pitchName(c.output), c.pitchCost.toFixed(3), c.harmonicCost.toFixed(3), c.score.toFixed(3)]) row.append(element('td', value));
    body.append(row);
  }
}
function renderContext() {
  const body = $('context'); body.replaceChildren();
  const context = sim.context();
  $('context-count').textContent = `${sim.held.size} sounding voices · ${sim.recent.length} remembered entries`;
  $('seed-note').textContent = seed.length ? `${seed.length} exact starting pitches were supplied by this fixture; all subsequent “on” events use the algorithm. Initial reference: 0 cents.` : 'Every note in this experiment is assigned by the algorithm.';
  for (const entry of context) {
    const row = element('tr');
    row.append(element('td', `${entry.id} · ${nodeName(entry.node)}`), element('td', pitchName(entry.output)), element('td', `${entry.status} · ${entry.weight.toFixed(3)}`));
    const bend = element('td'), action = element('td');
    if (entry.status === 'held') {
      const input = element('input'); Object.assign(input, { type: 'number', value: entry.bend, step: 1, min: -2400, max: 2400 });
      input.setAttribute('aria-label', `Player bend for ${entry.id} in cents`);
      input.addEventListener('change', () => guard(() => appendEvent({ type: 'bend', id: entry.id, cents: Number(input.value) }, `bend ${entry.id} ${Number(input.value)}`)));
      bend.append(input);
      const release = element('button', 'Release'); release.addEventListener('click', () => guard(() => appendEvent({ type: 'off', id: entry.id }, `off ${entry.id}`))); action.append(release);
    } else bend.textContent = '—';
    row.append(bend, action); body.append(row);
  }
}
function renderLattice(next) {
  const root = $('lattice'), width = Math.max(260, $('lattice-wrap').clientWidth), height = width < 450 ? 320 : 350;
  root.setAttribute('viewBox', `0 0 ${width} ${height}`); root.style.height = `${height}px`; root.replaceChildren();
  const allNodes = next.candidates.map(c => c.node);
  const slices = [...new Set(allNodes.map(n => n[2]))].sort((a, b) => a - b);
  let layer = Number($('slice').value || 0);
  if (!slices.includes(layer)) layer = slices.includes(0) ? 0 : slices[0];
  $('slice-field').hidden = slices.length <= 1;
  $('slice').replaceChildren(...slices.map(s => { const o = element('option', `${s >= 0 ? '+' : ''}${s}`); o.value = s; return o; })); $('slice').value = layer;
  const nodes = allNodes.filter(n => n[2] === layer);
  const context = sim.context();
  const minT = Math.min(...nodes.map(n => n[0])), maxT = Math.max(...nodes.map(n => n[0]));
  const minF = Math.min(...nodes.map(n => n[1])), maxF = Math.max(...nodes.map(n => n[1]));
  const left = 43, right = 25, top = 38, bottom = 53;
  const spacing = Math.min(58, (width - left - right) / Math.max(1, maxF - minF), (height - top - bottom) / Math.max(1, maxT - minT));
  const cx = (left + width - right) / 2, cy = (top + height - bottom) / 2;
  const x = n => cx + (n[1] - (minF + maxF) / 2) * spacing;
  const y = n => cy - (n[0] - (minT + maxT) / 2) * spacing;
  const nodeKeys = new Set(nodes.map(keyOf));
  for (const n of nodes) for (const step of [[1, 0, 0], [0, 1, 0]]) {
    const other = n.map((v, i) => v + step[i]);
    if (nodeKeys.has(keyOf(other))) root.append(svg('line', { x1: x(n), y1: y(n), x2: x(other), y2: y(other), class: 'edge' }));
  }
  root.append(svg('text', { x: width - 15, y: height - 6, 'text-anchor': 'end', class: 'axis-label' }, 'major thirds →'));
  root.append(svg('text', { x: 8, y: 14, class: 'axis-label' }, 'fifths ↑'));
  const stride = Math.max(1, Math.ceil(30 / spacing));
  for (let f = minF; f <= maxF; f += stride) root.append(svg('text', { x: x([0, f, layer]), y: height - 25, 'text-anchor': 'middle', class: 'axis-label' }, f));
  for (let t = minT; t <= maxT; t += stride) root.append(svg('text', { x: 25, y: y([t, 0, layer]) + 4, 'text-anchor': 'end', class: 'axis-label' }, t));
  const labelEvery = spacing >= 34;
  const radius = Math.max(3, Math.min(12, spacing * 0.23));
  for (const n of nodes) {
    const key = keyOf(n), held = context.some(v => v.status === 'held' && keyOf(v.node) === key);
    const remembered = context.some(v => v.status === 'released' && keyOf(v.node) === key);
    const reachable = reach?.keys.has(key), chosen = keyOf(next.winner.node) === key;
    const group = svg('g', { class: 'node-hit', role: 'button', tabindex: '0', 'aria-label': `${nodeName(n)}, coordinates ${n.join(', ')}, ${held ? 'sounding' : remembered ? 'remembered' : 'eligible'}${reachable ? ', reachable' : ''}`, transform: `translate(${x(n)},${y(n)})` });
    group.append(svg('circle', { r: Math.max(14, radius + 6), fill: 'transparent' }));
    if (reachable) group.append(svg('circle', { r: radius + 3, fill: 'none', stroke: 'var(--fg)', 'stroke-width': 1, opacity: .7 }));
    if (chosen || selected && keyOf(selected) === key) group.append(svg('circle', { r: radius + 6, fill: 'none', stroke: 'var(--selected)', 'stroke-width': chosen ? 2 : 1 }));
    group.append(svg('circle', { r: held || remembered ? radius : 2.5, fill: held ? 'var(--held)' : remembered ? 'var(--recent)' : 'var(--muted)', opacity: held ? 1 : remembered ? .65 : .55 }));
    if (labelEvery || held || chosen || selected && keyOf(selected) === key) group.append(svg('text', { y: -(radius + 9), 'text-anchor': 'middle' }, nodeName(n)));
    const select = () => { selected = n; renderSelected(); renderScores(next); renderLattice(next); };
    group.addEventListener('click', select); group.addEventListener('keydown', event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); select(); } }); root.append(group);
  }
  root.append(svg('desc', {}, `${nodes.length} eligible nodes on seventh layer ${layer}. ${reach ? reach.keys.size + ' reachable nodes across all layers in the inspected input range.' : 'Reachability calculation pending.'}`));
}
function renderSelected() {
  if (!selected) { $('node-detail').textContent = 'Select a lattice node to inspect its input ranges.'; return; }
  const bands = reach?.bands.filter(b => keyOf(b.node) === keyOf(selected)) || [];
  $('node-detail').textContent = `${nodeName(selected)} [${selected.join(', ')}] · ${reach ? bands.length ? bands.slice(0, 6).map(b => `${pitchName(b.low)} to ${pitchName(b.high)}`).join('; ') + (bands.length > 6 ? `; ${bands.length - 6} more intervals` : '') : reach.keys.has(keyOf(selected)) ? 'Reachable only at an exact boundary in this range.' : 'No winning inputs in this range.' : 'Input ranges are still computing.'}`;
}
function renderJourney() {
  const root = $('journey'), width = Math.max(260, root.clientWidth), height = 175;
  root.setAttribute('viewBox', `0 0 ${width} ${height}`); root.replaceChildren();
  const values = sim.history.map(v => v.correction);
  const min = Math.min(0, ...values), max = Math.max(0, ...values), span = Math.max(20, max - min);
  const low = min - span * .12, high = max + span * .12;
  const x = i => 65 + i * (width - 87) / Math.max(1, values.length - 1), y = v => 18 + (high - v) / (high - low) * 118;
  for (const value of [min, max].filter((v, i, arr) => arr.indexOf(v) === i)) {
    root.append(svg('line', { x1: 62, x2: width - 14, y1: y(value), y2: y(value), class: 'edge' }));
    root.append(svg('text', { x: 54, y: y(value) + 4, 'text-anchor': 'end', class: 'axis-label' }, value.toFixed(1)));
  }
  root.append(svg('text', { x: 8, y: 12, class: 'axis-label' }, 'cents'));
  root.append(svg('text', { x: width - 14, y: height - 4, 'text-anchor': 'end', class: 'axis-label' }, 'successive onsets →'));
  if (values.length) {
    root.append(svg('path', { d: values.map((v, i) => `${i ? 'L' : 'M'}${x(i)},${y(v)}`).join(' '), fill: 'none', stroke: 'var(--accent)', 'stroke-width': 1.8 }));
    root.append(svg('circle', { cx: x(values.length - 1), cy: y(values.at(-1)), r: 3.5, fill: 'var(--accent)' }));
  } else root.append(svg('text', { x: width / 2, y: 83, 'text-anchor': 'middle', class: 'axis-label' }, 'Play a note or step through the experiment.'));
}
function syncSound() {
  if (!$('sound').checked || !audio) {
    for (const sound of sounds.values()) { sound.gain.gain.setTargetAtTime(0, audio.currentTime, .015); sound.osc.stop(audio.currentTime + .1); }
    sounds.clear(); return;
  }
  for (const [id, sound] of sounds) if (!sim.held.has(id)) { sound.gain.gain.setTargetAtTime(0, audio.currentTime, .02); sound.osc.stop(audio.currentTime + .1); sounds.delete(id); }
  for (const [id, voice] of sim.held) {
    const frequency = 440 * 2 ** ((voice.output + voice.bend - 6900) / 1200);
    if (frequency < 10 || frequency > 20000) {
      const sound = sounds.get(id);
      if (sound) { sound.gain.gain.setTargetAtTime(0, audio.currentTime, .015); sound.osc.stop(audio.currentTime + .1); sounds.delete(id); }
      continue;
    }
    if (!sounds.has(id)) {
      const osc = audio.createOscillator(), gain = audio.createGain(); osc.type = 'sine'; gain.gain.value = 0;
      osc.connect(gain).connect(audio.destination); osc.start(); sounds.set(id, { osc, gain });
    }
    const sound = sounds.get(id); sound.osc.frequency.setValueAtTime(frequency, audio.currentTime);
    sound.gain.gain.setTargetAtTime(.08 / Math.max(1, sim.held.size), audio.currentTime, .015);
  }
}

for (const category of [...new Set(EXAMPLES.map(e => e.category))]) {
  const group = element('optgroup'); group.label = category;
  for (const e of EXAMPLES.filter(e => e.category === category)) { const option = element('option', e.name); option.value = e.id; group.append(option); }
  $('example').append(group);
}
$('example').addEventListener('change', () => guard(() => loadExample($('example').value)));
$('defaults').addEventListener('click', () => guard(() => changeSettings({ ...DEFAULTS, ...example.settings })));
$('axes').addEventListener('change', () => guard(() => changeSettings({ axes: Number($('axes').value) })));
$('silence').addEventListener('change', () => guard(() => changeSettings({ silence: Number($('silence').value) })));
$('reset-stop').addEventListener('change', () => guard(() => changeSettings({ resetStop: $('reset-stop').checked })));
$('reset-loop').addEventListener('change', () => guard(() => changeSettings({ resetLoop: $('reset-loop').checked })));
$('restart').addEventListener('click', () => guard(() => { stopPlayback(); seek(0); }));
$('step').addEventListener('click', () => guard(step));
$('finish').addEventListener('click', () => guard(() => { stopPlayback(); seek(events.length); }));
$('play').addEventListener('click', () => guard(togglePlay));
$('speed').addEventListener('change', () => { if (timer) { stopPlayback(); togglePlay(); } });
$('scrub').addEventListener('input', () => guard(() => { stopPlayback(); seek(Number($('scrub').value)); }));
for (const id of ['pitch', 'fine']) $(id).addEventListener('input', () => guard(() => { $('score-mode').value = 'next'; update(false); }));
$('attack').addEventListener('click', () => guard(() => {
  const pitch = probeInput(); let number = cursor + 1;
  while (events.some(e => e.id === `manual-${number}`)) number++;
  appendEvent({ type: 'on', pitch, id: `manual-${number}` }, `on ${pitch}c manual-${number}`);
}));
$('release-all').addEventListener('click', () => guard(() => appendEvent({ type: 'off', id: '*' }, 'off *')));
$('reset-context').addEventListener('click', () => guard(() => appendEvent({ type: 'reset' }, 'reset')));
$('score-mode').addEventListener('change', () => guard(() => renderScores(sim.evaluate(probeInput()))));
$('slice').addEventListener('change', () => guard(() => renderLattice(sim.evaluate(probeInput()))));
$('range-apply').addEventListener('click', () => guard(() => {
  const low = parsePitch($('range-low').value), high = parsePitch($('range-high').value);
  if (high <= low || high - low > 12000) throw new Error('Choose an ascending input range spanning at most ten octaves.');
  reachLow = low; reachHigh = high; requestReach();
}));
$('apply-program').addEventListener('click', () => guard(() => {
  const nextEvents = parseProgram($('program').value), old = events; events = nextEvents;
  try { freshState(settings, events.length); } catch (error) { events = old; throw error; }
  custom = true; $('expected').textContent = 'Edited sequence · inspect the outcomes to assess the model.'; stopPlayback(); seek(0);
}));
$('blank').addEventListener('click', () => guard(() => { seed = []; events = []; custom = true; $('program').value = ''; $('expected').textContent = 'Empty experiment · play any pitch to begin.'; stopPlayback(); seek(0); }));
$('run-checks').addEventListener('click', () => guard(() => {
  $('check-results').replaceChildren();
  let passed = 0;
  for (const e of EXAMPLES) {
    const result = runExample(e); if (result.passed) passed++;
    const row = element('div', undefined, 'check-result'); row.append(element('span', e.name), element('span', result.passed ? 'Target reached' : 'Mismatch', result.passed ? 'pass' : 'fail')); $('check-results').append(row);
  }
  $('check-results').prepend(element('p', `${passed} / ${EXAMPLES.length} musical targets reached with declared profiles.`, 'small'));
}));
$('theme').addEventListener('click', () => { const light = document.documentElement.dataset.theme !== 'light'; document.documentElement.dataset.theme = light ? 'light' : 'dark'; $('theme').textContent = light ? 'Dark theme' : 'Light theme'; });
$('sound').addEventListener('change', async () => {
  if ($('sound').checked) { audio ||= new AudioContext(); await audio.resume(); }
  syncSound();
});
$('export').addEventListener('click', () => guard(() => {
  const data = { version: 1, example: example.id, settings, seed, program: events.map(e => e.text).join('\n'), cursor, reachLow, reachHigh };
  const url = URL.createObjectURL(new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' }));
  const link = element('a'); link.href = url; link.download = 'tuning-experiment.json'; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
}));
$('import').addEventListener('change', async () => {
  const file = $('import').files[0]; if (!file) return;
  if (file.size > 1000000) { guard(() => { throw new Error('Experiment files must be under 1 MB.'); }); return; }
  const text = await file.text();
  guard(() => {
    const data = JSON.parse(text);
    if (data.version !== 1 || !Array.isArray(data.seed) || data.seed.length > 64 || typeof data.program !== 'string') throw new Error('Not a supported experiment file.');
    const newEvents = parseProgram(data.program), next = new Simulator(data.settings); next.seed(data.seed);
    if (!Number.isInteger(data.cursor) || data.cursor < 0 || data.cursor > newEvents.length) throw new Error('Invalid saved position.');
    const low = Number(data.reachLow), high = Number(data.reachHigh);
    if (!Number.isFinite(low) || !Number.isFinite(high) || high <= low || high - low > 12000) throw new Error('Invalid saved input range.');
    for (let i = 0; i < data.cursor; i++) next.event(newEvents[i]);
    stopPlayback(); settings = next.settings; seed = data.seed; events = newEvents; cursor = data.cursor; sim = next; custom = true;
    reachLow = low; reachHigh = high; $('range-low').value = `${low}c`; $('range-high').value = `${high}c`;
    example = EXAMPLES.find(e => e.id === data.example) || EXAMPLES[0]; $('example').value = example.id;
    $('program').value = data.program; $('description').textContent = 'Imported experiment · complete parameters, context and event sequence.'; $('expected').textContent = '';
    syncControls(); update(true);
  });
});
new ResizeObserver(() => guard(() => { renderLattice(sim.evaluate(probeInput())); renderJourney(); })).observe($('lattice-wrap'));
window.addEventListener('pagehide', () => { stopPlayback(); reachWorker?.terminate(); audio?.close(); });
guard(() => loadExample('thirds'));
