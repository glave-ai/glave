import { OneAtATime } from './reads';

describe('reads', () => {
  it('reads one at a time, and once more after for every ask meanwhile', async () => {
    const answers: (() => void)[] = [];
    let reads = 0;
    const reading = new OneAtATime(
      () =>
        new Promise((resolve) => {
          reads += 1;
          answers.push(resolve);
        }),
    );
    const first = reading.run();
    void reading.run();
    void reading.run();
    expect(reads).toBe(1);
    answers.shift()?.();
    await new Promise((resolve) => setTimeout(resolve));
    expect(reads).toBe(2);
    answers.shift()?.();
    await first;
    expect(reads).toBe(2);
    // Asked again once nothing is in flight, it reads at once.
    void reading.run();
    expect(reads).toBe(3);
  });

  it('reads again after a read that failed', async () => {
    let reads = 0;
    const reading = new OneAtATime(() => {
      reads += 1;
      return Promise.reject(new Error('unreached'));
    });
    await expect(reading.run()).rejects.toThrow('unreached');
    void reading.run().catch(() => undefined);
    expect(reads).toBe(2);
  });
});
