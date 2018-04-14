import { flatten } from './util';

function topologicalSort(dependencyGraph, onCyclesDetected) {
  const leaves = [];

  Object.keys(dependencyGraph).forEach(key => {
    if (dependencyGraph[key].size === 0) {
      delete dependencyGraph[key];
      leaves.push(key);
    }
  });

  const reverseDependencies = {};
  Object.keys(dependencyGraph).forEach(key => {
    const deps = dependencyGraph[key];
    deps.forEach(dep => {
      reverseDependencies[dep] = reverseDependencies[dep] || [];
      reverseDependencies[dep].push(key);
    });
  });

  const result = [];
  while (leaves.length) {
    const leaf = leaves.pop();
    result.push(leaf);
    (reverseDependencies[leaf] || []).forEach(key => {
      dependencyGraph[key].delete(leaf);
      if (dependencyGraph[key].size === 0) {
        delete dependencyGraph[key];
        leaves.push(key);
      }
    });
  }

  const unresolved = Object.keys(dependencyGraph);
  if (unresolved.length) {
    const collectCycles = (seen, key, curr) => {
      if (seen.has(key)) return [curr.concat([key]).join(' --> ')];

      return [...dependencyGraph[key]].map(nextKey =>
        collectCycles(new Set([...seen, key]), nextKey, [...curr, key])
      );
    };

    const cycles = flatten(
      unresolved.map(key => {
        return flatten(collectCycles(new Set(), key, []));
      })
    );

    onCyclesDetected(cycles);
    return;
  }

  return result;
}

export default topologicalSort;
