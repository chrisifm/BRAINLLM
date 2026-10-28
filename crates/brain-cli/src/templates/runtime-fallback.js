// Fallback runtime: only mounts the static tree. Replaced when runtime/brain.js exists.
export function mount(Comp, target, props = {}) {
  target.replaceChildren(Comp(props));
}
