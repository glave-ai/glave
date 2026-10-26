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
      [questions]="questions()"
      [answered]="answered()"
      (answer)="answers.push($event)"
    />
  `,
})
class HostComponent {
  readonly questions = signal<readonly GlzQuestion[]>(QUESTIONS);
  readonly answered = signal<readonly string[] | null>(null);
  readonly answers: (readonly string[])[] = [];
}

describe('GlzQuestionCardComponent', () => {
  it('collects one answer per question from choices and own words', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    const card = root.querySelector('glz-question-card');
    expect(document.getElementById(card?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'Needs you',
    );
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

  it('keeps what was picked and typed when the same questions come again, not new ones', async () => {
    const fixture = TestBed.createComponent(HostComponent);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    root.querySelectorAll<HTMLInputElement>('[data-testid="question-choice"] input')[0]?.click();
    const own = root.querySelectorAll<HTMLInputElement>('[data-testid="question-own"]')[2];
    if (own) {
      own.value = 'Keep it small';
      own.dispatchEvent(new Event('input'));
    }
    await fixture.whenStable();
    const picked = (): readonly boolean[] =>
      Array.from(
        root.querySelectorAll<HTMLInputElement>('[data-testid="question-choice"] input'),
        (choice) => choice.checked,
      );
    const typed = (): readonly string[] =>
      Array.from(
        root.querySelectorAll<HTMLInputElement>('[data-testid="question-own"]'),
        (input) => input.value,
      );
    // The same questions, read again as the thread rebuilds.
    fixture.componentInstance.questions.set(structuredClone(QUESTIONS));
    await fixture.whenStable();
    expect(picked()).toEqual([true, false, false, false]);
    expect(typed()).toEqual(['', '', 'Keep it small']);
    await expectAccessible(root);
    fixture.componentInstance.questions.set([{ question: 'Which port?' }]);
    await fixture.whenStable();
    expect(typed()).toEqual(['']);
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
    // Answered, it no longer needs the person: no label, and it rests.
    const card = root.querySelector('glz-question-card');
    expect(root.querySelector('.label')).toBeNull();
    expect(card?.classList).toContain('glz-question-card--answered');
    expect(document.getElementById(card?.getAttribute('aria-labelledby') ?? '')?.textContent).toBe(
      'Which database?',
    );
  });
});
