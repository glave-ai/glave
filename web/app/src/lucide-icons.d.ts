/**
 * One Lucide icon from its own module, which Lucide ships without types of its own. What the
 * app's first page loads (the side nav) imports its icons this way: through `lucide`'s index,
 * whose named re-exports the build follows, every icon a later page draws would load with it.
 */
declare module 'lucide/dist/esm/icons/*.mjs' {
  import type { IconNode } from 'lucide';

  const icon: IconNode;
  export default icon;
}
