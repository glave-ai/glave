import { InjectionToken } from '@angular/core';

import type { GlzDecisionLabels } from '../components/approval-card/approval-card.component';
import type { GlzReceiptStatus } from '../components/receipt/receipt.component';
import type { GlzRunStatusLabels } from '../components/steps/steps.component';
import type { GlzTrustAnswerLabels } from '../components/trust-card/trust-card.component';
import type { GlzPlanStatusLabels } from '../components/plan-card/plan-card.component';
import type { GlzMarkdownLabels } from '../lib/markdown-html';

/**
 * Every word Glaze writes itself, by component, each named as the input that replaces it for
 * one use where there is one. Glaze writes them in English; an app read in another language
 * gives them all in it through `GLZ_TEXT`.
 */
export interface GlzText {
  readonly approvalCard: {
    readonly label: string;
    readonly reasonLabel: string;
    readonly allowOnceLabel: string;
    readonly allowForThreadLabel: string;
    readonly denyLabel: string;
    readonly decidedLabels: GlzDecisionLabels;
  };
  readonly composer: {
    readonly placeholder: string;
    readonly label: string;
    readonly sendLabel: string;
    readonly stopLabel: string;
    readonly optionsLabel: string;
    readonly hint: string;
  };
  readonly confirmDialog: { readonly confirmLabel: string; readonly cancelLabel: string };
  readonly copyButton: {
    readonly label: string;
    readonly copiedLabel: string;
    readonly failedLabel: string;
  };
  readonly dialog: { readonly closeLabel: string };
  readonly markdown: GlzMarkdownLabels;
  readonly menuSelect: { readonly placeholder: string };
  readonly panelLayout: { readonly label: string; readonly resizeLabel: string };
  readonly planCard: {
    readonly label: string;
    readonly buildLabel: string;
    readonly changeLabel: string;
    readonly editLabel: string;
    readonly stepLabel: string;
    readonly removeLabel: string;
    readonly addLabel: string;
    readonly saveLabel: string;
    readonly cancelLabel: string;
    readonly statusLabels: GlzPlanStatusLabels;
  };
  readonly picker: {
    readonly placeholder: string;
    readonly searchLabel: string;
    readonly emptyText: string;
  };
  readonly questionCard: {
    readonly label: string;
    readonly ownWordsLabel: string;
    readonly submitLabel: string;
  };
  readonly receipt: { readonly statusLabels: Readonly<Record<GlzReceiptStatus, string>> };
  readonly sidenavLayout: { readonly label: string; readonly resizeLabel: string };
  readonly steps: { readonly statusLabels: GlzRunStatusLabels };
  readonly trustCard: {
    readonly label: string;
    readonly trustLabel: string;
    readonly notNowLabel: string;
    readonly answeredLabels: GlzTrustAnswerLabels;
  };
  readonly working: { readonly label: string };
}

/**
 * Glaze's words in the language an app is read in. Without it Glaze writes English. A
 * component's own input still replaces a word for that one use.
 *
 * ```ts
 * providers: [{ provide: GLZ_TEXT, useFactory: frenchWords }]
 * ```
 */
export const GLZ_TEXT = new InjectionToken<GlzText>('GLZ_TEXT');
