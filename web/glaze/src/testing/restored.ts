// Spec files that share a worker share its globals: Angular's unit-test builder runs Vitest
// without isolation. A stub one file leaves in place is what the next file's code calls, so a
// test passes or fails by which file ran before it (#1282). Each file puts back what it
// replaces; this checks it as each file ends, against what the file began with, and names what
// was left. Only names already there, on an object or its prototypes, count: what a library adds
// the first time it runs (axe's polyfills, a cache under a symbol) replaces nothing.

const WATCHED: readonly (readonly [string, object])[] = [
  ['window', globalThis],
  ['document', document],
  ['navigator', navigator],
  ['console', console],
  ['URL', URL],
  ['CSS', CSS],
  ['Storage.prototype', Storage.prototype],
  ['HTMLElement.prototype', HTMLElement.prototype],
];

/** Where `key` is found from `object` up its prototypes, and what it holds there. */
function found(object: object, key: string): readonly unknown[] {
  for (let at: object | null = object; at !== null; at = Reflect.getPrototypeOf(at)) {
    const descriptor = Reflect.getOwnPropertyDescriptor(at, key);
    if (descriptor !== undefined) {
      // Vitest's window keeps what is set on it behind accessors that stay the same.
      const read = object === globalThis && descriptor.get !== undefined ? onWindow(key) : null;
      return [at, descriptor.value, descriptor.get, descriptor.set, read];
    }
  }
  return [];
}

/** What `key` reads on the window now, or `null` where reading it throws. */
function onWindow(key: string): unknown {
  try {
    return Reflect.get(globalThis, key);
  } catch {
    return null;
  }
}

const before = new Map<string, readonly [object, string, readonly unknown[]]>();
for (const [name, object] of WATCHED) {
  for (let at: object | null = object; at !== null; at = Reflect.getPrototypeOf(at)) {
    for (const key of Object.getOwnPropertyNames(at)) {
      before.set(`${name}.${key}`, [object, key, found(object, key)]);
    }
  }
}

afterAll(() => {
  const replaced = [...before].flatMap(([name, [object, key, was]]) => {
    const is = found(object, key);
    const same = is.length === was.length && is.every((part, at) => Object.is(part, was[at]));
    return same ? [] : [name];
  });
  expect(replaced, 'globals this file replaced and never put back').toEqual([]);
});
