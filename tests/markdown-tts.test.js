import test from 'node:test';
import assert from 'node:assert/strict';

import { MarkdownSpeechStream, markdownToSpeechText } from '../src/lib/markdown.js';

test('markdownToSpeechText keeps prose while omitting code, equations, and tables', () => {
  const source = [
    '# Result',
    'The experiment converged after **ten** epochs. Its equation is $E = mc^2$.',
    String.raw`$$\int_0^1 x\,dx = \frac{1}{2}$$`,
    '| Metric | Value |\n| --- | ---: |\n| Accuracy | 99% |',
    'Read the [report](https://example.com/report) for context.',
    '```rust\nlet hidden = true;\n```',
    'Inline `hidden_code()` is not spoken.',
  ].join('\n\n');
  const speech = markdownToSpeechText(source);

  assert.match(speech, /The experiment converged after ten epochs\./);
  assert.match(speech, /Read the report for context\./);
  assert.doesNotMatch(speech, /E = mc|int_0|Metric|Accuracy|hidden|https?:|\||\$|```/);
});

test('markdownToSpeechText removes unfinished streamed display blocks', () => {
  const speech = markdownToSpeechText('Intro prose.\n\n```python\nprint("do not speak")');

  assert.equal(speech, 'Intro prose.');
});

test('MarkdownSpeechStream emits partial prose and suppresses streamed special blocks', () => {
  const stream = new MarkdownSpeechStream();
  const spoken = [];
  const feed = chunk => spoken.push(...stream.push(chunk));

  feed('Hello **world**. The equation is $E');
  feed('=mc^2$.\n| Metric | Value |\n| --- | --- |\n');
  feed('```python\nprint("hidden")\n```\n');
  feed('Useful prose starts now. Another sentence');
  spoken.push(...stream.finish());

  assert.match(spoken.join(' '), /Hello world\./);
  assert.match(spoken.join(' '), /Useful prose starts now\./);
  assert.match(spoken.join(' '), /Another sentence/);
  assert.doesNotMatch(spoken.join(' '), /E=mc|Metric|Value|print|hidden|\||\$|```|\*\*/);
  assert.ok(spoken.length >= 2, 'speech should be emitted before the stream finishes');
});
