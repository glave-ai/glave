import type { GlzColorOption } from '@glave/glaze';

/** The swatches a person chooses among, in the order offered. */
export function personSwatches(): readonly GlzColorOption[] {
  return [
    { value: '#0f766e', label: $localize`:@@accents.teal:Teal`, color: '#0f766e' },
    { value: '#2f7d32', label: $localize`:@@accents.green:Green`, color: '#2f7d32' },
    { value: '#d9580f', label: $localize`:@@accents.orange:Orange`, color: '#d9580f' },
    { value: '#c62828', label: $localize`:@@accents.red:Red`, color: '#c62828' },
    { value: '#6d3bd1', label: $localize`:@@accents.purple:Purple`, color: '#6d3bd1' },
    { value: '#f2c200', label: $localize`:@@accents.yellow:Yellow`, color: '#f2c200' },
  ];
}

/** The swatches an organization chooses among: a person's, with blue and lime. */
export function brandSwatches(): readonly GlzColorOption[] {
  return [
    { value: '#1558d6', label: $localize`:@@accents.blue:Blue`, color: '#1558d6' },
    { value: '#65a30d', label: $localize`:@@accents.lime:Lime`, color: '#65a30d' },
    ...personSwatches(),
  ];
}
