import test from 'node:test';
import assert from 'node:assert/strict';

import { protectMath } from '../src/lib/markdown.js';

test('protectMath renders inline and display LaTeX delimiters', () => {
  const result = protectMath(String.raw`Inline $E=mc^2$, \(a^2+b^2=c^2\), $$\int_0^1 x\,dx$$, and \[\frac{1}{2}\].`);

  assert.equal(result.rendered.length, 4);
  assert.ok(result.rendered.every(({ html }) => html.includes('katex')));
  assert.equal(result.rendered.filter(({ html }) => html.includes('katex-display')).length, 2);
  assert.doesNotMatch(result.markdown, /\$E=mc\^2\$/);
});

test('protectMath leaves currency and code examples untouched', () => {
  const source = "The price is $ 25. Use `$not_math$`.\n\n```js\nconst formula = '$also_not_math$';\n```";
  const result = protectMath(source);

  assert.equal(result.rendered.length, 0);
  assert.equal(result.markdown, source);
});

test('protectMath does not treat escaped dollar signs as equations', () => {
  const result = protectMath(String.raw`Cost: \$25. Equation: $x+y$.`);

  assert.equal(result.rendered.length, 1);
  assert.match(result.rendered[0].html, /katex/);
  assert.match(result.markdown, /\\\$25/);
});
