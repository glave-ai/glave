// Vitest's settings for the app's and Glaze's unit tests (angular.json, `runnerConfig`); the
// builder adds the rest. A test's time limit is a guard against a hang, not a speed limit: a
// test that takes a second on an idle machine can take many on a busy laptop. How many test
// files run at once is GLAVE_TEST_THREADS when set (`cargo xtask ci` sets it on a laptop),
// else Vitest's own choice.
import { defineConfig } from 'vitest/config';

const threads = Number(process.env['GLAVE_TEST_THREADS']) || undefined;

export default defineConfig({
  test: {
    testTimeout: 30_000,
    hookTimeout: 30_000,
    maxWorkers: threads,
  },
});
