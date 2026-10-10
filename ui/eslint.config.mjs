// Copied from Northstar ui/eslint.config.mjs at e9c1589.
// Changed: the restricted-import rule names Carbon Charts instead of Northstar's graph libraries, and allows them only through import(); there is no generated client to ignore.
import js from '@eslint/js';
import globals from 'globals';
import reactHooks from 'eslint-plugin-react-hooks';
import reactRefresh from 'eslint-plugin-react-refresh';
import tseslint from 'typescript-eslint';

/// Carbon Charts, which may only ever be loaded lazily. Northstar's charts stylesheet alone is
/// 241 kB and its charts code most of a megabyte with d3 inside; a static import from a module
/// the shell reaches would put them in the entry chunk every page loads before it draws, and one
/// from a screen would load them with that screen whether it draws a chart or not. So a chart
/// arrives by `import('@carbon/charts-react')` (or a lazy component that holds it), and nowhere
/// else. `import type` stays allowed everywhere, because it is erased. scripts/check-chunks.mjs
/// proves the same thing of the build's output, which no rule about source can answer for the
/// bundler.
const CHARTS = ['@carbon/charts', '@carbon/charts/*', '@carbon/charts-react', '@carbon/charts-react/*'];
const CHARTS_ONLY_LAZILY = 'Carbon Charts load only through import(), so they stay out of the entry chunk and out of every screen that draws no chart.';
/// Specs load whatever they test, statically: they are in no chunk.
const TESTS = ['**/*.spec.{ts,tsx}'];

export default tseslint.config(
  { ignores: ['dist'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: { ecmaVersion: 2023, globals: globals.browser },
    plugins: { 'react-hooks': reactHooks, 'react-refresh': reactRefresh },
    rules: {
      ...reactHooks.configs.recommended.rules,
      'react-refresh/only-export-components': ['warn', { allowConstantExport: true }],
      '@typescript-eslint/no-unused-vars': ['error', { argsIgnorePattern: '^_' }],
      '@typescript-eslint/no-restricted-imports': [
        'error',
        { patterns: [{ group: CHARTS, allowTypeImports: true, message: CHARTS_ONLY_LAZILY }] },
      ],
    },
  },
  {
    files: TESTS,
    rules: { '@typescript-eslint/no-restricted-imports': 'off' },
  },
);
