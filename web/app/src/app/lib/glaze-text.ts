import type { GlzText } from '@glave/glaze';

/**
 * Glaze's own words as the app's messages, for a translation to give them (`GLZ_TEXT`). Read
 * only when the app is read in one: in English Glaze writes these words itself, so the first
 * load carries none of them twice.
 */
export function glazeText(): GlzText {
  const choose = $localize`:@@glaze.choose:Choose`;
  return {
    ...cards(),
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
      done: $localize`:@@glaze.markdown.done:Done`,
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
    receipt: {
      statusLabels: {
        running: $localize`:@@glaze.receipt.running:Running`,
        done: $localize`:@@glaze.receipt.done:Done`,
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
        done: $localize`:@@glaze.steps.done:Done`,
        failed: $localize`:@@glaze.steps.failed:failed`,
        stopped: $localize`:@@glaze.steps.stopped:stopped`,
        running: $localize`:@@glaze.steps.running:working`,
        waiting: $localize`:@@glaze.steps.waiting:waiting`,
      },
    },
    working: { label: $localize`:@@glaze.working:Working…` },
  };
}

/** The words of the cards that need the person: an approval, questions, a project's trust, a plan. */
function cards(): Pick<GlzText, 'approvalCard' | 'questionCard' | 'trustCard' | 'planCard'> {
  const needsYou = $localize`:@@glaze.needsYou:Needs you`;
  return {
    planCard: {
      label: needsYou,
      buildLabel: $localize`:@@glaze.plan.build:Build it`,
      changeLabel: $localize`:@@glaze.plan.change:Change it`,
      editLabel: $localize`:@@glaze.plan.edit:Edit`,
      stepLabel: $localize`:@@glaze.plan.step:Step`,
      removeLabel: $localize`:@@glaze.plan.remove:Remove step`,
      addLabel: $localize`:@@glaze.plan.add:Add a step`,
      saveLabel: $localize`:@@glaze.plan.save:Save`,
      cancelLabel: $localize`:@@glaze.plan.cancel:Cancel`,
      statusLabels: {
        pending: $localize`:@@glaze.plan.pending:To do`,
        in_progress: $localize`:@@glaze.plan.doing:Doing`,
        completed: $localize`:@@glaze.plan.done:Done`,
      },
    },
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
    questionCard: {
      label: needsYou,
      ownWordsLabel: $localize`:@@glaze.question.ownWords:In your own words`,
      submitLabel: $localize`:@@glaze.question.answer:Answer`,
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
  };
}
