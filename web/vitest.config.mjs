// Vitest's settings for the app's and Glaze's unit tests (angular.json, `runnerConfig`); the
// builder adds the rest. A test's time limit is a guard against a hang, not a speed limit: a
// test that takes a second on an idle machine can take many on a busy laptop.
import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    testTimeout: 30_000,
    hookTimeout: 30_000,
  },
});
