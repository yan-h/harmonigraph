import { Simulator } from './model.mjs';
self.onmessage = ({ data }) => {
  try {
    const sim = new Simulator(data.settings);
    sim.reference = data.reference;
    sim.context = () => data.context;
    const start = performance.now();
    const result = sim.reachability(data.low, data.high);
    self.postMessage({ result, milliseconds: performance.now() - start });
  } catch (error) { self.postMessage({ error: error.message }); }
};
