import type { ModelInfo, RawEvent } from '@glave/client';
import type { GlzMenuOption } from '@glave/glaze';

/** What marks the server's default model in the picker. */
export const DEFAULT_HINT = 'Default';

/** A model as the thread names it: what to call it, and whether it is Hope. */
export interface ModelLabel {
  readonly name: string;
  readonly hope: boolean;
}

/** The thread's own choice of model: the latest `model.chosen`'s, or `null` for the default. */
export function chosenModel(events: readonly RawEvent[]): string | null {
  const chosen = events.filter((event) => event.type === 'model.chosen').at(-1);
  const model = chosen?.data['model'];
  return typeof model === 'string' && model !== '' ? model : null;
}

/** The server's default model, from `model/list`, or `null` before it is read. */
export function defaultModel(models: readonly ModelInfo[]): string | null {
  return (models.find((model) => model.default) ?? models.at(0))?.id ?? null;
}

/** The picker's options: each model by its name, the default marked, Hope only when `hope`. */
export function modelOptions(models: readonly ModelInfo[]): readonly GlzMenuOption[] {
  return models.map((model) => ({
    value: model.id,
    label: nameOf(model),
    hope: model.hope,
    ...(model.default ? { hint: DEFAULT_HINT } : {}),
  }));
}

/**
 * How the thread names the model `id` (as `model.responded` or `turn.started` names it): its
 * configured name and Hope's mark from `model/list`, else the id as it is, never Hope.
 * `fallback` names a reply whose model is not known.
 */
export function modelLabel(
  models: readonly ModelInfo[],
  id: string | null,
  fallback: string,
): ModelLabel {
  if (id === null) {
    return { name: fallback, hope: false };
  }
  const found = models.find((model) => model.id === id);
  return found === undefined
    ? { name: id, hope: false }
    : { name: nameOf(found), hope: found.hope };
}

function nameOf(model: ModelInfo): string {
  const name = model.name?.trim() ?? '';
  return name === '' ? model.id : name;
}
