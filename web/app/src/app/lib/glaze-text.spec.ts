import type { Type } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import {
  GlzApprovalCardComponent,
  GlzComposerComponent,
  GlzConfirmDialogComponent,
  GlzCopyButtonComponent,
  GlzMenuSelectComponent,
  GlzPanelLayoutComponent,
  GlzPickerComponent,
  GlzQuestionCardComponent,
  GlzReasoningComponent,
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
  ['questionCard', GlzQuestionCardComponent],
  ['reasoning', GlzReasoningComponent],
  ['sidenavLayout', GlzSidenavLayoutComponent],
  ['steps', GlzStepsComponent],
  ['trustCard', GlzTrustCardComponent],
  ['working', GlzWorkingComponent],
];

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
});
