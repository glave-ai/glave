// Lint for web/: typescript-eslint strict and stylistic, type-checked; angular-eslint's
// recommended TypeScript and template rules with its accessibility set; and template rules
// that keep logic out of templates (AGENTS.md, "Web client").
import js from '@eslint/js';
import angular from 'angular-eslint';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  {
    ignores: [
      'dist/',
      'out-tsc/',
      '.angular/',
      'node_modules/',
      'test-results/',
      'coverage/',
      'playwright-report/',
      // Generated from the session API's schema by `cargo xtask schema`.
      'client/src/generated/',
    ],
  },
  {
    files: ['**/*.ts'],
    extends: [
      js.configs.recommended,
      ...tseslint.configs.strictTypeChecked,
      ...tseslint.configs.stylisticTypeChecked,
      ...angular.configs.tsRecommended,
    ],
    languageOptions: {
      parserOptions: { project: ['./tsconfig.all.json'], tsconfigRootDir: import.meta.dirname },
    },
    processor: angular.processInlineTemplates,
    rules: {
      '@angular-eslint/component-selector': [
        'error',
        { type: ['element', 'attribute'], prefix: ['glz', 'app'], style: 'kebab-case' },
      ],
      '@angular-eslint/directive-selector': [
        'error',
        { type: 'attribute', prefix: ['glz', 'app'], style: 'camelCase' },
      ],
      '@angular-eslint/prefer-on-push-component-change-detection': 'error',
      '@angular-eslint/prefer-signals': 'error',
      '@angular-eslint/no-input-rename': 'off',
      '@angular-eslint/no-output-rename': 'off',
      '@typescript-eslint/explicit-function-return-type': ['error', { allowExpressions: true }],
      '@typescript-eslint/explicit-member-accessibility': ['error', { accessibility: 'no-public' }],
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/prefer-readonly': 'error',
      '@typescript-eslint/consistent-type-imports': ['error', { fixStyle: 'inline-type-imports' }],
      '@typescript-eslint/no-extraneous-class': ['error', { allowWithDecorator: true }],
      '@typescript-eslint/no-confusing-void-expression': ['error', { ignoreArrowShorthand: true }],
      'prefer-const': 'error',
      'no-console': 'error',
    },
  },
  {
    // A message is read where it is used, in a function or as a class is made, never as its
    // module is read: a translation is loaded before the app starts, after main.ts's imports.
    files: ['app/src/**/*.ts'],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          selector:
            "TaggedTemplateExpression[tag.name='$localize']:not(:function TaggedTemplateExpression, PropertyDefinition[static=false] TaggedTemplateExpression)",
          message:
            'Read a message where it is used, in a function or a class field: at a module’s top level it is read before a translation loads.',
        },
        {
          // Each message keeps its id when its English changes, so translations keep it too.
          selector:
            "TaggedTemplateExpression[tag.name='$localize'] > TemplateLiteral > TemplateElement:first-child:not([value.raw=/^:@@[A-Za-z][\\w.]*:/])",
          message: 'Give the message a stable id: $localize`:@@area.key:Text`.',
        },
      ],
    },
  },
  {
    // Glaze stays app-independent: its words come from its inputs and GLZ_TEXT.
    files: ['glaze/**/*.ts'],
    rules: {
      'no-restricted-globals': [
        'error',
        { name: '$localize', message: 'Glaze takes its words from inputs and GLZ_TEXT.' },
      ],
    },
  },
  {
    // Components on native elements (`button[glzButton]`) keep the element's own semantics; the
    // selector rule cannot read element-and-attribute selectors. Markdown renders into any
    // element (`[glzMarkdown]`), the way a directive is used.
    files: [
      'glaze/src/components/button/button.component.ts',
      'glaze/src/components/icon-button/icon-button.component.ts',
      'glaze/src/components/markdown/markdown.component.ts',
      'glaze/src/components/nav-item/nav-item.component.ts',
    ],
    rules: { '@angular-eslint/component-selector': 'off' },
  },
  {
    files: ['**/*.spec.ts', 'e2e/**/*.ts', '**/testing/**/*.ts'],
    rules: {
      '@typescript-eslint/no-non-null-assertion': 'off',
      '@typescript-eslint/no-unsafe-type-assertion': 'off',
      // TestBed's fixture.nativeElement is typed any; tests read it as the element it is.
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/no-unsafe-argument': 'off',
      '@typescript-eslint/no-unsafe-member-access': 'off',
      '@typescript-eslint/unbound-method': 'off',
      // Query helpers take the element type, as querySelector does.
      '@typescript-eslint/no-unnecessary-type-parameters': 'off',
      '@angular-eslint/prefer-on-push-component-change-detection': 'off',
    },
  },
  {
    files: ['**/*.html'],
    extends: [...angular.configs.templateRecommended, ...angular.configs.templateAccessibility],
    rules: {
      // Logic stays in the controller: conditions are one signal or field, no `&&`, `||`, `?:`.
      '@angular-eslint/template/conditional-complexity': ['error', { maxComplexity: 1 }],
      '@angular-eslint/template/cyclomatic-complexity': ['error', { maxComplexity: 12 }],
      '@angular-eslint/template/no-any': 'error',
      '@angular-eslint/template/no-inline-styles': 'error',
      '@angular-eslint/template/no-interpolation-in-attributes': 'error',
      '@angular-eslint/template/prefer-control-flow': 'error',
      '@angular-eslint/template/prefer-self-closing-tags': 'error',
      '@angular-eslint/template/prefer-at-empty': 'error',
      '@angular-eslint/template/prefer-at-else': 'error',
      '@angular-eslint/template/prefer-template-literal': 'error',
      '@angular-eslint/template/no-non-null-assertion': 'error',
      '@angular-eslint/template/eqeqeq': 'error',
      '@angular-eslint/template/button-has-type': 'off',
      // It cannot see content a Glaze component renders into the element (an icon and label);
      // axe checks every link and button has a name in the rendered page instead.
      '@angular-eslint/template/elements-content': 'off',
    },
  },
);
