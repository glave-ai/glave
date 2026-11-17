/**
 * A read of something the server says, run one at a time: at once, or, while one runs, once
 * more after it, however often it was asked meanwhile, so a run of events that each ask for it
 * costs the server two reads.
 */
export class OneAtATime {
  /** Whether a read is in flight. */
  private reading = false;
  /** Whether a read was asked for since the one in flight began. */
  private again = false;

  constructor(private readonly read: () => Promise<void>) {}

  /** Reads now, or once more after the read in flight. */
  async run(): Promise<void> {
    if (this.reading) {
      this.again = true;
      return;
    }
    this.reading = true;
    this.again = false;
    try {
      do {
        await this.read();
      } while (this.asked());
    } finally {
      this.reading = false;
    }
  }

  /** Whether a read was asked for since the last began, taking it as begun. */
  private asked(): boolean {
    const asked = this.again;
    this.again = false;
    return asked;
  }
}
