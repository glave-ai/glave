import { ChangeDetectionStrategy, Component, type Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  GLZ_TEXT,
  GlzApprovalCardComponent,
  GlzComposerComponent,
  GlzConfirmDialogComponent,
  GlzCopyButtonComponent,
  GlzDialogComponent,
  GlzMarkdownComponent,
  GlzMenuSelectComponent,
  GlzPanelLayoutComponent,
  GlzPickerComponent,
  GlzPlanCardComponent,
  GlzQuestionCardComponent,
  GlzServiceCardComponent,
  GlzReceiptComponent,
  GlzSidenavLayoutComponent,
  GlzStepsComponent,
  type GlzText,
  GlzTrustCardComponent,
  GlzWorkingComponent,
} from '@glave/glaze';

import { glazeText } from './glaze-text';

/** Each part of Glaze with its words as inputs, and the component that has them. */
const PARTS: readonly (readonly [keyof GlzText, Type<object>])[] = [
  ['approvalCard', GlzApprovalCardComponent],
  ['composer', GlzComposerComponent],
  ['confirmDialog', GlzConfirmDialogComponent],
  ['copyButton', GlzCopyButtonComponent],
  ['menuSelect', GlzMenuSelectComponent],
  ['panelLayout', GlzPanelLayoutComponent],
  ['picker', GlzPickerComponent],
  ['planCard', GlzPlanCardComponent],
  ['questionCard', GlzQuestionCardComponent],
  ['serviceCard', GlzServiceCardComponent],
  ['sidenavLayout', GlzSidenavLayoutComponent],
  ['steps', GlzStepsComponent],
  ['trustCard', GlzTrustCardComponent],
  ['working', GlzWorkingComponent],
];

/** The parts whose words are no inputs, each showing every word it has. */
@Component({
  imports: [GlzDialogComponent, GlzMarkdownComponent, GlzReceiptComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (status of statuses; track status) {
      <glz-receipt summary="cargo test" [status]="status" />
    }
    <glz-dialog heading="Rename" [open]="true" />
    <div [glzMarkdown]="markdown"></div>
  `,
})
class InsideComponent {
  protected readonly statuses = ['running', 'done', 'failed', 'declined'] as const;
  protected readonly markdown = [
    '- [x] Fixed',
    '- [ ] Documented',
    '',
    '[The guide](https://example.com/guide) and ![a chart](https://example.com/chart.png)',
    '',
    '```text',
    'ok',
    '```',
  ].join('\n');
}

/** What `InsideComponent` renders, with `text` as Glaze's words or with Glaze's own. */
async function inside(text: GlzText | null): Promise<string> {
  TestBed.resetTestingModule();
  TestBed.configureTestingModule({
    providers: text === null ? [] : [{ provide: GLZ_TEXT, useValue: text }],
  });
  const fixture = TestBed.createComponent(InsideComponent);
  await fixture.whenStable();
  const root: HTMLElement = fixture.nativeElement;
  return root.innerHTML;
}

describe('glazeText', () => {
  it('says in English what Glaze itself says, so a translation starts from the same words', () => {
    const app = glazeText();
    for (const [part, component] of PARTS) {
      const own = TestBed.createComponent(component).componentInstance as Readonly<
        Record<string, () => unknown>
      >;
      for (const [input, words] of Object.entries(app[part])) {
        expect([part, input, own[input]?.()]).toEqual([part, input, words]);
      }
    }
  });

  it('draws the parts whose words are no inputs as Glaze itself does, in English', async () => {
    const own = await inside(null);
    expect(own).toContain('Copy code');
    expect(await inside(glazeText())).toBe(own);
  });
});
