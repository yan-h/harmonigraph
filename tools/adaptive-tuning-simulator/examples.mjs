import { AXES, Simulator, parsePitch, parseProgram, keyOf } from './model.mjs';

const seed = (id, input, node) => ({ id, input, node });
const chain = [seed('c', 'C3', [0, 0, 0]), seed('g', 'G3', [1, 0, 0]), seed('d', 'D4', [2, 0, 0]), seed('a', 'A4', [3, 0, 0])];
const minor = [seed('c', 'C3', [0, 0, 0]), seed('eb', 'Eb3', [1, -1, 0]), seed('g', 'G3', [1, 0, 0])];
export function thirdCycleProgram(chords = 19) {
  return Array.from({ length: chords }, (_, i) => {
    const root = 4800 + (i % 3) * 400;
    return `${i ? 'off *\nwait 0.15\n' : ''}on ${root}c root-${i}\non ${root + 400}c third-${i}\non ${root + 700}c fifth-${i}`;
  }).join('\n');
}
export const EXAMPLES = [
  { id: 'thirds', name: 'Keep moving right', category: 'Journey',
    description: 'C → E → A♭ major, repeated. Each chord releases before the next. The roots should advance one major-third step each time.',
    program: thirdCycleProgram(),
    check: sim => sim.history.filter(v => v.id.startsWith('root-')).every((v, i) => keyOf(v.node) === `0,${i},0`),
    expected: '19 roots at [0, 0, 0] through [0, 18, 0]; returned C descends six dieses.' },
  { id: 'fifths-high', name: 'Fifth chain · high E', category: 'Register',
    description: 'The first four onsets establish C3–G3–D4–A4. High E5 should continue that chain.',
    program: 'on C3 c\non G3 g\non D4 d\non A4 a\non E5 e', expectedNode: [4, 0, 0], expected: 'E at four fifth steps: 81/16 above C3.' },
  { id: 'fifths-low', name: 'Fifth chain · low E', category: 'Register',
    description: 'Identical fifth-chain context, but the incoming E is now near C3.',
    program: 'on C3 c\non G3 g\non D4 d\non A4 a\non E3 e', expectedNode: [0, 1, 0], expected: 'E at one major-third step: 5/4 above C3.' },
  { id: 'minor', name: 'Minor harmony · 9/5', category: 'Context', seed: minor,
    description: 'Exact just C–E♭–G is provided as a controlled starting context. Play the same 12-TET B♭ used in the C–F experiment.',
    program: 'on Bb3 bb', expectedNode: [2, -1, 0], expected: 'B♭ at 9/5, a fifth above E♭.' },
  { id: 'minor-far', name: 'Distant E♭ · still 9/5', category: 'Register', seed: minor.map(v => v.id === 'eb' ? { ...v, input: 'Eb6' } : v),
    description: 'E♭ is three octaves farther away. Its influence weakens, but the lattice preference should survive.',
    program: 'on Bb3 bb', expectedNode: [2, -1, 0], expected: 'B♭ remains at 9/5.' },
  { id: 'fourths', name: 'C–F harmony · 16/9', category: 'Context', seed: [seed('c', 'C3', [0, 0, 0]), seed('f', 'F3', [-1, 0, 0])],
    description: 'An exact C–F context receives the same 12-TET B♭ input as the minor harmony.',
    program: 'on Bb3 bb', expectedNode: [-2, 0, 0], expected: 'B♭ at 16/9, a fourth above F.' },
  { id: 'held-e', name: 'Sounding low E', category: 'Memory',
    description: 'A low just E is added to the fifth chain and stays sounding as high E arrives.',
    program: 'on C3 c\non G3 g\non D4 d\non A4 a\non E3 low\non E5 high', expectedNode: [0, 1, 0], expected: 'Baseline weights favour the sounding just E.' },
  { id: 'released-e', name: 'Released low E', category: 'Memory',
    description: 'Release the low E immediately before high E. It remains remembered, but loses held-note weight.',
    program: 'on C3 c\non G3 g\non D4 d\non A4 a\non E3 low\noff low\non E5 high', expectedNode: [4, 0, 0], expected: 'High E continues the fifth chain.' },
  { id: 'precise-e', name: 'Intentional Pythagorean E', category: 'Precision', settings: { harmonic: 2 },
    seed: [...chain, seed('low', 'E3', [0, 1, 0])],
    description: 'Starting pitches and zero reference are seeded explicitly. Lower harmonic preference allows precise input to choose a different E from the one sounding.',
    program: `on ${(4800 + 4 * AXES[0]).toFixed(9)}c high`, expectedNode: [4, 0, 0], expected: 'Precise input selects the Pythagorean E.' },
  { id: 'nominal-e', name: 'Same context · ordinary E', category: 'Precision', settings: { harmonic: 2 },
    seed: [...chain, seed('low', 'E3', [0, 1, 0])],
    description: 'Same seeded context and parameters as the precise-input experiment, now with a 12-TET E5.',
    program: 'on E5 high', expectedNode: [0, 1, 0], expected: 'Ordinary E5 favours the sounding just E.' },
  { id: 'lone-c-low', name: 'Lone C · low E', category: 'Register', seed: [seed('c', 'C3', [0, 0, 0])],
    description: 'One C reference should favour a just E.', program: 'on E3 e', expectedNode: [0, 1, 0], expected: '5/4 pitch class.' },
  { id: 'lone-c-high', name: 'Lone C · high E', category: 'Register', seed: [seed('c', 'C3', [0, 0, 0])],
    description: 'Moving the input much higher should not invent a fifth-chain preference without that context.',
    program: 'on E6 e', expectedNode: [0, 1, 0], expected: 'Still the 5/4 pitch class.' },
  { id: 'pump', name: 'A conventional comma pump', category: 'Journey',
    description: 'C major → F major → D minor → G major → C major. Common tones remain held; the original C releases before the return.',
    program: 'on C3 c\non E3 e\non G3 g\noff e\noff g\non F3 f\non A3 a\noff c\non D3 d\noff f\noff a\non G3 g2\non B3 b\noff d\noff b\non C3 return\non E3 e2',
    check: sim => keyOf(sim.history.find(v => v.id === 'return')?.node || []) === '-4,1,0',
    expected: 'Returned C is 80/81 of the original, 21.51 cents lower.' },
  { id: 'pedal', name: 'An old pedal releases last', category: 'Memory',
    description: 'C is struck a second before E and G and released after them. A release does not restart its age, so it stays behind them in released memory.',
    program: 'on C3 c\nwait 1\non E3 e\non G3 g\noff e\noff g\noff c',
    check: sim => sim.recent.map(e => e.id).join() === 'g,e,c', expected: 'C, struck first, is the oldest released contribution.' },
  { id: 'bend', name: 'Player bend · onset memory', category: 'Expression',
    description: 'Bend E by 30 cents. Its audible pitch moves, but its onset context and adaptive correction stay fixed. Enable sound to listen.',
    program: 'on C3 c\non E3 e\nbend e 30\non G3 g',
    check: sim => sim.held.get('e')?.bend === 30 && keyOf(sim.held.get('e')?.node || []) === '0,1,0', expected: 'E has a +30-cent bend and an unchanged onset position.' },
];
export function runExample(example, overrides = {}, transpose = 0) {
  const sim = new Simulator({ ...example.settings, ...overrides });
  sim.seed((example.seed || []).map(v => ({ ...v, input: (typeof v.input === 'string' ? parsePitch(v.input) : v.input) + transpose,
    ...(v.output === undefined ? {} : { output: v.output + transpose }) })));
  for (const event of parseProgram(example.program)) sim.event(event.type === 'on' ? { ...event, pitch: event.pitch + transpose } : event);
  const passed = example.check ? example.check(sim) : keyOf(sim.history.at(-1)?.node || []) === keyOf(example.expectedNode);
  return { sim, passed, example };
}
