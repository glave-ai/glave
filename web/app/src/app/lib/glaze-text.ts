import type { GlzText } from '@glave/glaze';

/**
 * Glaze's own words as the app's messages, for a translation to give them (`GLZ_TEXT`). Read
 * only when the app is read in one: in English Glaze writes these words itself, so the first
 * load carries none of them twice.
 */
export function glazeText(): GlzText {
  const needsYou = $localize`:@@glaze.needsYou:Needs you`;
  const done = $localize`:@@glaze.done:Done`;
  const choose = $localize`:@@glaze.choose:Choose`;
  return {
    approvalCard: {
      label: needsYou,
      reasonLabel: $localize`:@@glaze.approval.reason:The model says`,
      allowOnceLabel: $localize`:@@glaze.approval.allowOnce:Allow once`,
      allowForThreadLabel: $localize`:@@glaze.approval.allowThread:Allow for this thread`,
      denyLabel: $localize`:@@glaze.approval.deny:Deny`,
      decidedLabels: {
        allow_once: $localize`:@@glaze.approval.allowedOnce:Allowed once`,
        allow_for_thread: $localize`:@@glaze.approval.allowedThread:Allowed for this thread`,
        deny: $localize`:@@glaze.approval.denied:Denied`,
      },
    },
    composer: {
      placeholder: $localize`:@@glaze.composer.placeholder:Write a message`,
      label: $localize`:@@glaze.composer.label:Message`,
      sendLabel: $localize`:@@glaze.composer.send:Send`,
      stopLabel: $localize`:@@glaze.composer.stop:Stop`,
      optionsLabel: $localize`:@@glaze.composer.options:Options`,
      hint: $localize`:@@glaze.composer.hint:Enter to send, Shift+Enter for a new line`,
    },
    confirmDialog: {
      confirmLabel: $localize`:@@glaze.confirm.continue:Continue`,
      cancelLabel: $localize`:@@glaze.confirm.cancel:Cancel`,
    },
    copyButton: {
      label: $localize`:@@glaze.copy.label:Copy`,
      copiedLabel: $localize`:@@glaze.copy.copied:Copied`,
      failedLabel: $localize`:@@glaze.copy.failed:Could not copy`,
    },
    dialog: { closeLabel: $localize`:@@glaze.dialog.close:Close` },
    markdown: {
      newTab: $localize`:@@glaze.markdown.newTab:(opens in a new tab)`,
      image: $localize`:@@glaze.markdown.image:Image`,
      done,
      notDone: $localize`:@@glaze.markdown.notDone:Not done`,
      copyCode: $localize`:@@glaze.markdown.copyCode:Copy code`,
    },
    menuSelect: { placeholder: choose },
    panelLayout: {
      label: $localize`:@@glaze.panel.label:Panel`,
      resizeLabel: $localize`:@@glaze.panel.resize:Resize the panel`,
    },
    picker: {
      placeholder: choose,
      searchLabel: $localize`:@@glaze.picker.search:Search`,
      emptyText: $localize`:@@glaze.picker.empty:No matches`,
    },
    questionCard: {
      label: needsYou,
      ownWordsLabel: $localize`:@@glaze.question.ownWords:In your own words`,
      submitLabel: $localize`:@@glaze.question.answer:Answer`,
    },
    reasoning: {
      label: $localize`:@@glaze.reasoning.label:Reasoning`,
      thinkingLabel: $localize`:@@glaze.reasoning.thinking:Thinking…`,
    },
    receipt: {
      statusLabels: {
        running: $localize`:@@glaze.receipt.running:Running`,
        done,
        failed: $localize`:@@glaze.receipt.failed:Failed`,
        declined: $localize`:@@glaze.receipt.declined:Declined`,
      },
    },
    sidenavLayout: {
      label: $localize`:@@glaze.sidenav.label:Navigation`,
      resizeLabel: $localize`:@@glaze.sidenav.resize:Resize the navigation`,
    },
    steps: {
      statusLabels: {
        done,
        failed: $localize`:@@glaze.steps.failed:failed`,
        stopped: $localize`:@@glaze.steps.stopped:stopped`,
        running: $localize`:@@glaze.steps.running:working`,
        waiting: $localize`:@@glaze.steps.waiting:waiting`,
      },
    },
    trustCard: {
      label: needsYou,
      trustLabel: $localize`:@@glaze.trust.trust:Trust this project`,
      notNowLabel: $localize`:@@glaze.trust.notNow:Not now`,
      answeredLabels: {
        trust: $localize`:@@glaze.trust.trusted:Trusted`,
        not_now: $localize`:@@glaze.trust.keptOff:Kept off for now`,
      },
    },
    working: { label: $localize`:@@glaze.working:Working…` },
  };
}
