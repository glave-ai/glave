import { TestBed } from '@angular/core/testing';
import { expectAccessible } from '@glave/glaze/testing';

import { RenameDialogComponent } from './rename-dialog.component';

function input(root: HTMLElement): HTMLInputElement {
  const found = root.querySelector<HTMLInputElement>('[data-testid="rename-input"]');
  if (found === null) {
    throw new Error('no input');
  }
  return found;
}

function type(root: HTMLElement, value: string): void {
  input(root).value = value;
  input(root).dispatchEvent(new Event('input'));
}

describe('RenameDialogComponent', () => {
  it('starts from the title and emits the new one, trimmed', async () => {
    const fixture = TestBed.createComponent(RenameDialogComponent);
    fixture.componentRef.setInput('title', 'Plan');
    fixture.componentRef.setInput('open', true);
    const renamed: string[] = [];
    fixture.componentInstance.renamed.subscribe((title) => renamed.push(title));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(input(root).value).toBe('Plan');
    expect(input(root).maxLength).toBe(200);
    await expectAccessible(root);
    type(root, '  Launch plan  ');
    await fixture.whenStable();
    root.querySelector<HTMLButtonElement>('[data-testid="rename-save"]')?.click();
    expect(renamed).toEqual(['Launch plan']);
    expect(fixture.componentInstance.open()).toBe(false);
  });

  it('will not save an empty title, and saves on Enter', async () => {
    const fixture = TestBed.createComponent(RenameDialogComponent);
    fixture.componentRef.setInput('open', true);
    const renamed: string[] = [];
    fixture.componentInstance.renamed.subscribe((title) => renamed.push(title));
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    type(root, '   ');
    await fixture.whenStable();
    const save = root.querySelector<HTMLButtonElement>('[data-testid="rename-save"]');
    expect(save?.disabled).toBe(true);
    root.querySelector('form')?.dispatchEvent(new Event('submit', { cancelable: true }));
    expect(renamed).toEqual([]);
    type(root, 'Notes');
    root.querySelector('form')?.dispatchEvent(new Event('submit', { cancelable: true }));
    expect(renamed).toEqual(['Notes']);
  });

  it('renames a project by its name', async () => {
    const fixture = TestBed.createComponent(RenameDialogComponent);
    fixture.componentRef.setInput('kind', 'project');
    fixture.componentRef.setInput('title', 'glave');
    fixture.componentRef.setInput('open', true);
    await fixture.whenStable();
    const root: HTMLElement = fixture.nativeElement;
    expect(root.querySelector('[data-testid="rename"]')?.textContent).toContain(
      'Rename the project',
    );
    expect(root.querySelector('label')?.textContent).toContain('Name');
    expect(input(root).value).toBe('glave');
    await expectAccessible(root);
  });

  it('closes on Cancel', async () => {
    const fixture = TestBed.createComponent(RenameDialogComponent);
    fixture.componentRef.setInput('open', true);
    await fixture.whenStable();
    (fixture.nativeElement as HTMLElement)
      .querySelector<HTMLButtonElement>('[data-testid="rename-cancel"]')
      ?.click();
    expect(fixture.componentInstance.open()).toBe(false);
  });
});
