import { ChangeDetectionStrategy, Component, signal } from '@angular/core';
import { ComponentFixtureAutoDetect, TestBed } from '@angular/core/testing';
import { Settings, X } from 'lucide';

import { GlzButtonComponent } from '../components/button/button.component';
import { GlzCheckboxComponent } from '../components/checkbox/checkbox.component';
import { GlzComposerComponent } from '../components/composer/composer.component';
import { GlzFormFieldComponent } from '../components/form-field/form-field.component';
import { GlzIconButtonComponent } from '../components/icon-button/icon-button.component';
import { GlzIconComponent } from '../components/icon/icon.component';
import { GlzMenuButtonComponent } from '../components/menu-button/menu-button.component';
import {
  type GlzMenuOption,
  GlzMenuSelectComponent,
} from '../components/menu-select/menu-select.component';
import { GlzNavItemComponent } from '../components/nav-item/nav-item.component';
import { GlzPickerComponent } from '../components/picker/picker.component';
import { GlzRadioGroupComponent } from '../components/radio-group/radio-group.component';
import { GlzRadioComponent } from '../components/radio/radio.component';
import { GlzReceiptComponent } from '../components/receipt/receipt.component';
import { GlzStepsComponent } from '../components/steps/steps.component';
import { GlzThoughtComponent } from '../components/thought/thought.component';
import { GlzInputDirective } from '../directives/input.directive';
import { expectAccessible } from '../testing/axe';
import type { GlzOption } from './options';

/** Each of Glaze's controls, named by bindings, as an app names them in its words. */
@Component({
  imports: [
    GlzButtonComponent,
    GlzCheckboxComponent,
    GlzComposerComponent,
    GlzFormFieldComponent,
    GlzIconButtonComponent,
    GlzIconComponent,
    GlzInputDirective,
    GlzMenuButtonComponent,
    GlzMenuSelectComponent,
    GlzNavItemComponent,
    GlzPickerComponent,
    GlzRadioComponent,
    GlzRadioGroupComponent,
    GlzReceiptComponent,
    GlzStepsComponent,
    GlzThoughtComponent,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <button glzButton type="button">{{ words.save }}</button>
    <button glzIconButton type="button" [label]="words.close">
      <glz-icon [icon]="icons.close" />
    </button>
    <a glzNavItem href="/settings" [icon]="icons.settings">{{ words.settings }}</a>
    <glz-menu-button [label]="words.options" [actions]="actions" />
    <glz-menu-select [label]="words.mode" [options]="options" [(value)]="value" />
    <glz-picker [label]="words.model" [options]="options" [(value)]="value" />
    <glz-composer [label]="words.message" />
    <glz-form-field [label]="words.name"><input glzInput /></glz-form-field>
    <glz-checkbox>{{ words.remember }}</glz-checkbox>
    <glz-radio-group [label]="words.mode" [(value)]="value">
      <glz-radio value="ask">{{ words.ask }}</glz-radio>
    </glz-radio-group>
    <glz-thought [label]="words.thought" [headline]="words.why" />
    <glz-receipt [summary]="words.ran">{{ words.output }}</glz-receipt>
    <glz-steps [steps]="words.steps">{{ words.step }}</glz-steps>
  `,
})
class NamedComponent {
  protected readonly words = {
    save: 'Save',
    close: 'Close',
    settings: 'Settings',
    options: 'Options for Atlas',
    mode: 'Mode',
    model: 'Model',
    message: 'Message',
    name: 'Name',
    remember: 'Remember this choice',
    ask: 'Ask',
    thought: 'Thought for 2 seconds',
    why: 'The tests import the parser.',
    ran: 'Ran the tests',
    output: '3 passed',
    steps: '2 steps',
    step: 'Read the parser',
  } as const;
  protected readonly icons = { close: X, settings: Settings };
  protected readonly actions: readonly GlzOption[] = [{ value: 'rename', label: 'Rename' }];
  protected readonly options: readonly GlzMenuOption[] = [{ value: 'ask', label: 'Ask' }];
  readonly value = signal<string | null>('ask');
}

/** What a person reaches in `root`: buttons, links, fields and disclosures. */
function controls(root: HTMLElement): readonly HTMLElement[] {
  return Array.from(root.querySelectorAll<HTMLElement>('button, a, input, textarea, summary'));
}

/**
 * What names `control`, roughly as a screen reader finds it: its label, else the labels of a
 * field, else its text.
 */
function name(control: HTMLElement): string {
  const label = control.getAttribute('aria-label');
  if (label !== null) {
    return label;
  }
  if (control instanceof HTMLInputElement || control instanceof HTMLTextAreaElement) {
    return Array.from(control.labels ?? [], (each) => each.textContent).join(' ');
  }
  return control.textContent;
}

function named(control: HTMLElement): boolean {
  return name(control).trim() !== '';
}

describe('INERT_UNTIL_BOUND', () => {
  beforeEach(() => {
    // Nothing binds the view until the test does, as a view Angular adds outside change
    // detection (a deferred block, a route's page) waits for the next one.
    TestBed.configureTestingModule({
      providers: [{ provide: ComponentFixtureAutoDetect, useValue: false }],
    });
  });

  it('keeps each control out of reach on its first render, unnamed, then hands it over named', async () => {
    const fixture = TestBed.createComponent(NamedComponent);
    const root: HTMLElement = fixture.nativeElement;
    // Made, not yet bound: none has a name, and none can be reached.
    expect(controls(root)).toHaveLength(15);
    expect(controls(root).filter(named)).toEqual([]);
    expect(controls(root).filter((control) => control.closest('[inert]') === null)).toEqual([]);
    await expectAccessible(root);

    // The change detection that follows binds it.
    fixture.changeDetectorRef.markForCheck();
    fixture.detectChanges();
    await fixture.whenStable();
    expect(root.querySelectorAll('[inert]')).toHaveLength(0);
    expect(controls(root).filter((control) => !named(control))).toEqual([]);
    await expectAccessible(root);
  });
});
