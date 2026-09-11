import test from 'node:test';
import assert from 'node:assert/strict';
import { AXES, Simulator, keyOf, parsePitch, parseProgram, nodeName } from './model.mjs';
import { EXAMPLES, runExample, thirdCycleProgram } from './examples.mjs';

test('agreed musical examples use declared fixture settings and actual context', () => {
  for (const example of EXAMPLES) {
    const result = runExample(example);
    assert.ok(result.passed, `${example.name}: got ${keyOf(result.sim.history.at(-1)?.node || [])}, wanted ${example.expected}`);
  }
});

test('101 major-third cycles keep moving after displacement exceeds an octave', () => {
  const sim = new Simulator();
  for (const event of parseProgram(thirdCycleProgram(304))) sim.event(event);
  const roots = sim.history.filter(v => v.id.startsWith('root-'));
  assert.equal(roots.length, 304);
  for (const [i, v] of roots.entries()) {
    assert.deepEqual(v.node, [0, i, 0]);
    assert.ok(Math.abs(v.correction - i * (AXES[1] - 400)) < 1e-7);
  }
  assert.ok(roots.at(-1).correction < -4000, 'fixture must travel beyond three octaves of drift');
});

test('transpose complete performances by an octave without changing lattice choices', () => {
  for (const example of EXAMPLES) {
    const a = runExample(example).sim, b = runExample(example, {}, 1200).sim;
    assert.deepEqual(b.history.map(v => v.node), a.history.map(v => v.node), example.name);
    a.history.forEach((v, i) => assert.ok(Math.abs(b.history[i].output - v.output - 1200) < 1e-7, example.name));
  }
});

test('repeated voices refresh memory; octave duplicates and comma shifts remain distinct', () => {
  const sim = new Simulator({ memory: 2 });
  sim.seed([{ id: 'a', input: 4800, node: [0, 0, 0] }, { id: 'b', input: 4800, node: [0, 0, 0] },
    { id: 'octave', input: 6000, node: [0, 0, 0] }, { id: 'comma', input: 4800, node: [0, 3, 0] }]);
  assert.equal(sim.context().length, 3);
  sim.off('a'); sim.off('b');
  assert.equal(sim.recent.length, 1);
  assert.equal(sim.recent[0].id, 'b');
  sim.off('octave'); sim.off('comma');
  assert.deepEqual(sim.recent.map(v => v.id), ['comma', 'octave']);
  assert.equal(sim.recent[0].node[1], 3);
});

test('release recency does not weaken on time alone, while configured silence resets', () => {
  const sim = new Simulator(); sim.on(4800, 'c'); sim.on(5200, 'e'); sim.off('e'); sim.off('c');
  assert.equal(sim.recent[0].id, 'c');
  const before = structuredClone(sim.recent), shift = sim.reference;
  sim.wait(60);
  assert.deepEqual(sim.recent, before); assert.equal(sim.reference, shift);
  const timed = new Simulator({ silence: 2 }); timed.on(5200, 'e'); timed.off('e');
  timed.wait(1.9); assert.equal(timed.recent.length, 1);
  timed.wait(0.1); assert.equal(timed.recent.length, 0); assert.equal(timed.reference, 0);
});

test('a held note struck long before the newest weighs less, and waiting changes no held weight', () => {
  const program = 'on E3 low\nwait 3\non C3 c\non G3 g\non D4 d\non A4 a\non E5 high';
  const high = heldHalfLife => {
    const sim = new Simulator({ heldHalfLife });
    for (const event of parseProgram(program)) sim.event(event);
    return keyOf(sim.history.at(-1).node);
  };
  assert.equal(high(0), '0,1,0', 'weighed alike, the sounding just E decides');
  assert.equal(high(1), '4,0,0', 'three half-lives older, it no longer outvotes the chain');
  const sim = new Simulator(); sim.on(4800, 'c'); sim.on(5200, 'e');
  const before = sim.context();
  sim.wait(60);
  assert.deepEqual(sim.context(), before);
});

test('player bends and later decisions never modify onset context or frozen correction', () => {
  const sim = new Simulator(); sim.on(4800, 'c'); sim.on(5200, 'e');
  const before = { ...sim.held.get('e') }, rank = sim.evaluate(5500);
  sim.bend('e', 37);
  assert.equal(sim.held.get('e').output, before.output);
  assert.equal(sim.held.get('e').correction, before.correction);
  assert.deepEqual(sim.evaluate(5500).candidates, rank.candidates);
  sim.on(5500, 'g'); assert.equal(sim.held.get('e').correction, before.correction);
});

test('hard eligibility and allowed axes constrain every candidate before pitch scoring', () => {
  for (const axes of [1, 2, 3]) {
    const sim = new Simulator({ radius: 2, axes });
    const candidates = sim.candidates();
    for (const n of candidates) {
      assert.ok(n.reduce((sum, x) => sum + Math.abs(x), 0) <= 2);
      for (let i = axes; i < 3; i++) assert.equal(n[i], 0);
    }
    assert.equal(candidates.length, [0, 5, 13, 25][axes]);
    assert.ok(!candidates.some(n => keyOf(n) === '12,0,0'));
  }
});

test('winner intervals agree with direct selection across pitches and registers', () => {
  for (const axes of [2, 3]) {
    const sim = new Simulator({ axes, radius: 2 });
    sim.on(4800, 'c'); sim.on(5200, 'e'); sim.off('e');
    const reach = sim.reachability(3600, 8400);
    assert.ok(reach.bands.length > 20, 'fixture must include multiple registers');
    for (const band of reach.bands) {
      const middle = (band.low + band.high) / 2;
      assert.equal(keyOf(sim.evaluate(middle).winner.node), keyOf(band.node));
    }
    for (let input = 3600.137; input < 8400; input += 13.719) {
      const band = reach.bands.find(b => input >= b.low && input < b.high);
      assert.ok(band, `uncovered input ${input}`);
      assert.equal(keyOf(sim.evaluate(input).winner.node), keyOf(band.node));
    }
  }
  const sim = new Simulator({ harmonic: 4 });
  const endpoint = sim.reachability(5399.9, 5400);
  assert.ok(endpoint.keys.has(keyOf(sim.evaluate(5400).winner.node)), 'include an endpoint-only winner');
  assert.ok(sim.reachability(5000, 5000.00000001).keys.size > 0, 'tiny accepted ranges retain a winner');
});

test('transport settings clear context without changing a held voice correction on a loop', () => {
  for (const resetLoop of [false, true]) {
    const sim = new Simulator({ resetLoop }); sim.on(5200, 'e');
    const correction = sim.held.get('e').correction;
    sim.transport('loop');
    assert.equal(sim.reference, resetLoop ? 0 : correction);
    assert.equal(sim.held.get('e').correction, correction);
  }
  const sim = new Simulator({ resetStop: true }); sim.on(5200, 'e'); sim.transport('stop');
  assert.equal(sim.held.size, 0); assert.equal(sim.recent.length, 0); assert.equal(sim.reference, 0);
});

test('continuous input parser accepts fine pitches and reports invalid programs', () => {
  assert.equal(parsePitch('E5+7.82c'), 7607.82);
  assert.equal(parsePitch('76.0782m'), 7607.82);
  assert.equal(parsePitch('Eb3'), 5100);
  assert.throws(() => parseProgram('on C3 c\nwait nope'), /Line 2/);
  assert.throws(() => parseProgram('on C3'), /Line 1/);
  assert.throws(() => new Simulator({ pitchScale: 0 }), /pitchScale/);
  assert.equal(nodeName([0, 3, 0]), 'B♯');
  assert.equal(nodeName([0, 18, 0]), 'D♯10', 'long journeys retain unwrapped diatonic spelling');
});
