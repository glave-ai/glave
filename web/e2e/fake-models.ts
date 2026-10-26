import type { ModelInfo } from '@glave/client';

/** The default model, described as OpenRouter's catalogue describes it. */
export const DEFAULT_MODEL = 'anthropic/claude-haiku-5.5';

/**
 * The models the fake server runs: its default, Hope, two more, and a long catalogue like
 * OpenRouter's, which the picker must search and scroll.
 */
export const MODELS: readonly ModelInfo[] = [
  {
    id: DEFAULT_MODEL,
    name: 'Claude Haiku 5.5',
    maker: 'Anthropic',
    default: true,
    providers: ['OpenRouter'],
    context: 1_000_000,
    price: { input: 100_000, output: 500_000 },
  },
  {
    id: 'hope-12b',
    name: 'Hope',
    default: false,
    reasoning: { efforts: ['high', 'low', 'none'], default: 'low' },
  },
  { id: 'openai/gpt-6-luna', name: 'GPT-6 Luna', maker: 'OpenAI', default: false },
  {
    id: 'deepseek/deepseek-v4.1-flash',
    name: 'DeepSeek V4.1 Flash',
    maker: 'DeepSeek',
    default: false,
  },
  ...Array.from({ length: 200 }, (_, index) => ({
    id: `lab-${String(index % 20)}/model-${String(index)}`,
    name: `Model ${String(index)}`,
    maker: `Lab ${String(index % 20)}`,
    default: false,
  })),
];
