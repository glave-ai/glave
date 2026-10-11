import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';

import type { GlzQuestion } from '../../lib/questions';
import { expectAccessible } from '../../testing/axe';
import { GlzQuestionCardComponent } from './question-card.component';

const QUESTIONS: readonly GlzQuestion[] = [
  {
    question: 'Which database?',
    options: [{ label: 'SQLite', description: 'One file' }, { label: 'Postgres' }],
  },
  { question: 'Which checks?', options: [{ label: 'Lint' }, { label: 'Tests' }], multiple: true },
  { question: 'Anything else?' },
];

@Component({
  imports: [GlzQuestionCardComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <glz-question-card
      testId="question"
      [questions]="questions"
      [answered]="answered()"
      (answer)="answers.push($event)"
    />
  `,
})
class HostComponent {
  readonly questions = QUESTIONS;
  readonly answered = signal<readonly string[] | null>(null);
  readonly answers: (readonly string[])[] = [];
}

describe('GlzQuestionCardComponent', () => {
  it('collects one answer per question from choices and own words', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const submit = root.querySelector<HTMLButtonElement>('[data-testid="question-submit"]');
    expect(submit?.disabled).toBe(true);
    await expectAccessible(root);

    const choices = root.querySelectorAll<HTMLInputElement>(
      '[data-testid="question-choice"] input',
    );
    choices[1]?.click();
    choices[2]?.click();
    choices[3]?.click();
    const own = root.querySelectorAll<HTMLInputElement>('[data-testid="question-own"]');
    const words = own[2];
    if (words) {
      words.value = 'Keep it small';
      words.dispatchEvent(new Event('input'));
    }
    await fixture.whenStable();
    expect(submit?.disabled).toBe(false);
    submit?.click();
    expect(fixture.componentInstance.answers).toEqual([
      ['Postgres', 'Lint, Tests', 'Keep it small'],
    ]);
  });

  it('shows the answers in place once answered', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    fixture.componentInstance.answered.set(['SQLite', 'Tests', 'No']);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const answers = Array.from(root.querySelectorAll('[data-testid="question-answer"]')).map(
      (answer) => answer.textContent,
    );
    expect(answers).toEqual(['SQLite', 'Tests', 'No']);
    expect(root.querySelector('[data-testid="question-submit"]')).toBeNull();
  });
});
