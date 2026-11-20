import { Service, inject } from '@angular/core';
import type { ScheduleId, ScheduleInfo } from '@glave/client';

import { SessionService } from './session.service';

/**
 * The person's schedules changed from the app: paused, resumed or removed, each answered as the
 * server has it. Loaded only with what shows a schedule, never with the app's first page.
 */
@Service()
export class SchedulesService {
  private readonly session = inject(SessionService);

  /** Pauses `schedule` (`paused`) or resumes it; throws what the server refused it with. */
  async pause(schedule: ScheduleId, paused: boolean): Promise<ScheduleInfo> {
    const answer = await this.session.call('schedule/pause', { schedule, paused });
    return answer.schedule;
  }

  /** Removes `schedule`, keeping its chat; throws what the server refused it with. */
  async remove(schedule: ScheduleId): Promise<void> {
    await this.session.call('schedule/delete', { schedule });
  }
}
